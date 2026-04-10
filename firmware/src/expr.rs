//! Tiny stack-machine bytecode evaluator for dynamic MIDI mappings.
//!
//! Expressions are compiled to bytecode in the browser UI and evaluated
//! on the microcontroller.  The VM operates on `u16` values internally so
//! that intermediate arithmetic (especially multiply) can exceed 127
//! without premature saturation.  Only the **final** result is clamped to
//! the MIDI range 0-127.
//!
//! # Bytecode format
//!
//! Each instruction is one opcode byte, optionally followed by an immediate:
//!
//! | Opcode | Immediate | Stack effect | Description            |
//! |--------|-----------|--------------|------------------------|
//! | 0x01   | u8        | -> val       | Push literal           |
//! | 0x02   | u8 (idx)  | -> val       | Load pot value         |
//! | 0x03   | --        | -> val       | Load LDR value         |
//! | 0x04   | --        | -> val       | Load accel X           |
//! | 0x05   | --        | -> val       | Load accel Y           |
//! | 0x10   | --        | a b -> a+b   | Saturating add         |
//! | 0x11   | --        | a b -> a-b   | Saturating subtract    |
//! | 0x12   | --        | a b -> a*b   | Saturating multiply    |
//! | 0x13   | --        | a b -> a/b   | Integer divide         |
//! | 0x14   | --        | a b -> min   | Minimum                |
//! | 0x15   | --        | a b -> max   | Maximum                |
//! | 0x16   | --        | a b c -> r   | Clamp a to [b, c]      |
//! | 0x17   | --        | a b c -> r   | Lerp from a to b by c  |
//! | 0x18   | u8 (mode) | a b -> r     | Scale-quantise b from root a |
//! | 0x20   | --        | a b c d -> r | If a > b then c else d |

use crate::config::{MAX_EXPR, MAX_POTS};

/// Inputs available to expressions — a snapshot of live controller state.
pub struct ExprInputs {
    pub pots: [u8; MAX_POTS],
    pub ldr: u8,
    pub accel_x: u8,
    pub accel_y: u8,
}

// Opcodes
const OP_PUSH: u8 = 0x01;
const OP_LOAD_POT: u8 = 0x02;
const OP_LOAD_LDR: u8 = 0x03;
const OP_LOAD_ACCEL_X: u8 = 0x04;
const OP_LOAD_ACCEL_Y: u8 = 0x05;
const OP_ADD: u8 = 0x10;
const OP_SUB: u8 = 0x11;
const OP_MUL: u8 = 0x12;
const OP_DIV: u8 = 0x13;
const OP_MIN: u8 = 0x14;
const OP_MAX: u8 = 0x15;
const OP_CLAMP: u8 = 0x16;
const OP_LERP: u8 = 0x17;
const OP_SCALE: u8 = 0x18;
const OP_IF_GT: u8 = 0x20;

const STACK_SIZE: usize = 8;

/// Scale intervals (semitones from root) for each of the 7 diatonic modes,
/// ordered brightest to darkest: Lydian, Ionian, Mixolydian, Dorian,
/// Aeolian, Phrygian, Locrian.
const SCALE_INTERVALS: [[u8; 7]; 7] = [
    [0, 2, 4, 6, 7, 9, 11], // 0 = Lydian (default)
    [0, 2, 4, 5, 7, 9, 11], // 1 = Ionian (Major)
    [0, 2, 4, 5, 7, 9, 10], // 2 = Mixolydian
    [0, 2, 3, 5, 7, 9, 10], // 3 = Dorian
    [0, 2, 3, 5, 7, 8, 10], // 4 = Aeolian (Minor)
    [0, 1, 3, 5, 7, 8, 10], // 5 = Phrygian
    [0, 1, 3, 5, 6, 8, 10], // 6 = Locrian
];

/// Total number of scale degrees available (18 degrees ≈ 2.5 octaves).
const SCALE_DEGREES: u16 = 18;

/// Push a value onto the stack if there is room.
const fn stack_push(stack: &mut [u16; STACK_SIZE], sp: &mut usize, val: u16) {
    if *sp < STACK_SIZE {
        stack[*sp] = val;
        *sp += 1;
    }
}

/// Pop two values and apply a binary operation, pushing the result.
fn binop(stack: &mut [u16; STACK_SIZE], sp: &mut usize, f: impl FnOnce(u16, u16) -> u16) {
    if *sp >= 2 {
        *sp -= 1;
        stack[*sp - 1] = f(stack[*sp - 1], stack[*sp]);
    }
}

/// Pop three values and apply a ternary operation, pushing the result.
fn triop(stack: &mut [u16; STACK_SIZE], sp: &mut usize, f: impl FnOnce(u16, u16, u16) -> u16) {
    if *sp >= 3 {
        *sp -= 2;
        stack[*sp - 1] = f(stack[*sp - 1], stack[*sp], stack[*sp + 1]);
    }
}

/// Evaluate a bytecode program.  Returns `fallback` if the program is empty
/// or malformed.
pub fn eval(program: &[u8; MAX_EXPR], len: u8, inputs: &ExprInputs, fallback: u8) -> u8 {
    let len = len as usize;
    if len == 0 {
        return fallback;
    }
    let code = &program[..len.min(MAX_EXPR)];
    let mut stack = [0u16; STACK_SIZE];
    let mut sp: usize = 0;
    let mut pc: usize = 0;

    while pc < code.len() {
        match code[pc] {
            OP_PUSH => {
                pc += 1;
                if pc >= code.len() {
                    break;
                }
                stack_push(&mut stack, &mut sp, u16::from(code[pc]));
            }
            OP_LOAD_POT => {
                pc += 1;
                if pc >= code.len() {
                    break;
                }
                let idx = code[pc] as usize;
                let v = if idx < inputs.pots.len() {
                    inputs.pots[idx]
                } else {
                    0
                };
                stack_push(&mut stack, &mut sp, u16::from(v));
            }
            OP_LOAD_LDR => stack_push(&mut stack, &mut sp, u16::from(inputs.ldr)),
            OP_LOAD_ACCEL_X => stack_push(&mut stack, &mut sp, u16::from(inputs.accel_x)),
            OP_LOAD_ACCEL_Y => stack_push(&mut stack, &mut sp, u16::from(inputs.accel_y)),
            OP_ADD => binop(&mut stack, &mut sp, u16::saturating_add),
            OP_SUB => binop(&mut stack, &mut sp, u16::saturating_sub),
            OP_MUL => {
                binop(&mut stack, &mut sp, |a, b| {
                    let r = u32::from(a).saturating_mul(u32::from(b));
                    #[allow(clippy::cast_possible_truncation)]
                    let val = r.min(u16::MAX as u32) as u16;
                    val
                });
            }
            OP_DIV => {
                binop(&mut stack, &mut sp, |a, b| {
                    a.checked_div(b).unwrap_or(u16::MAX)
                });
            }
            OP_MIN => binop(&mut stack, &mut sp, u16::min),
            OP_MAX => binop(&mut stack, &mut sp, u16::max),
            OP_CLAMP => triop(&mut stack, &mut sp, |val, lo, hi| {
                let (lo, hi) = (lo.min(hi), lo.max(hi));
                val.clamp(lo, hi)
            }),
            OP_LERP => triop(&mut stack, &mut sp, |a, b, t| {
                // lerp(a, b, t): linearly interpolate from a to b, where t=0 → a, t=127 → b
                let a = i32::from(a);
                let b = i32::from(b);
                let t = i32::from(t);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let val = (a + (b - a) * t / 127).clamp(0, i32::from(u16::MAX)) as u16;
                val
            }),
            OP_SCALE => {
                // scale(root, position, mode): quantise a 0-127 position to a
                // diatonic scale rooted at `root`.
                // The mode byte follows the opcode in the bytecode stream.
                pc += 1;
                if pc >= code.len() {
                    break;
                }
                let mode_idx = code[pc] as usize;
                binop(&mut stack, &mut sp, |root, pos| {
                    let intervals = if mode_idx < SCALE_INTERVALS.len() {
                        &SCALE_INTERVALS[mode_idx]
                    } else {
                        &SCALE_INTERVALS[0] // fallback to Lydian
                    };
                    // Map position (0-127) to a scale degree (0..SCALE_DEGREES-1)
                    let degree = u16::from(pos.min(127) as u8) * SCALE_DEGREES / 128;
                    let octave = degree / 7;
                    let step = (degree % 7) as usize;
                    let semitones = octave * 12 + u16::from(intervals[step]);
                    let note = root.saturating_add(semitones);
                    // Clamp here (not just at final output) so scale() always
                    // produces a valid MIDI note, even when composed with
                    // further arithmetic like `scale(60, pot0) + 12`.
                    note.min(127)
                });
            }
            OP_IF_GT
                // stack: ... test_val threshold then_val else_val
                if sp >= 4 =>
            {
                let a = stack[sp - 4]; // test value
                let b = stack[sp - 3]; // threshold
                let c = stack[sp - 2]; // then value
                let d = stack[sp - 1]; // else value
                sp -= 3;
                stack[sp - 1] = if a > b { c } else { d };
            }
            _ => {} // unknown opcode: skip
        }
        pc += 1;
    }

    if sp > 0 {
        #[allow(clippy::cast_possible_truncation)]
        let result = stack[sp - 1].min(127) as u8;
        result
    } else {
        fallback
    }
}
