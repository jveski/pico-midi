// Expression compiler: infix text → stack-machine bytecode.
//
// Grammar:
//   expr     = ternary
//   ternary  = compare (('?' compare ':' compare)?)
//   compare  = additive (('>' additive)?)
//   additive       = multiplicative (('+' | '-') multiplicative)*
//   multiplicative = atom (('*' | '/') atom)*
//   atom     = NUMBER | VARIABLE | call | '(' expr ')'
//   call     = ('min' | 'max') '(' expr ',' expr ')'
//            | ('clamp' | 'lerp') '(' expr ',' expr ',' expr ')'
//            | 'scale' '(' expr ',' expr (',' MODE)? ')'
//   MODE     = NUMBER | 'lydian' | 'ionian' | 'major' | 'mixolydian'
//            | 'dorian' | 'aeolian' | 'minor' | 'phrygian' | 'locrian'
//
// Variables: pot0-pot2, ldr, accel_x, accel_y
//
// Examples:
//   pot0 + 24
//   pot0 > 64 ? 60 : 48
//   min(pot0, 100)
//   clamp(pot0, 20, 100)
//   lerp(36, 84, pot0)
//   pot0 * 2
//   scale(60, pot0)
//   scale(48, pot0, minor)

import { MAX_EXPR } from "./protocol.js";

const OP_PUSH       = 0x01;
const OP_LOAD_POT   = 0x02;
const OP_LOAD_LDR   = 0x03;
const OP_LOAD_AX    = 0x04;
const OP_LOAD_AY    = 0x05;
const OP_ADD        = 0x10;
const OP_SUB        = 0x11;
const OP_MUL        = 0x12;
const OP_DIV        = 0x13;
const OP_MIN        = 0x14;
const OP_MAX        = 0x15;
const OP_CLAMP      = 0x16;
const OP_LERP       = 0x17;
const OP_SCALE      = 0x18;
const OP_IF_GT      = 0x20;

// Scale intervals for each mode (ordered brightest to darkest).
// Each sub-array has 7 semitone offsets from the root.
// The actual interval lookup happens on the microcontroller
// (see SCALE_INTERVALS in firmware/src/expr.rs).
const SCALE_MODES = [
  [0, 2, 4, 6, 7, 9, 11], // 0 = Lydian (default)
  [0, 2, 4, 5, 7, 9, 11], // 1 = Ionian (Major)
  [0, 2, 4, 5, 7, 9, 10], // 2 = Mixolydian
  [0, 2, 3, 5, 7, 9, 10], // 3 = Dorian
  [0, 2, 3, 5, 7, 8, 10], // 4 = Aeolian (Minor)
  [0, 1, 3, 5, 7, 8, 10], // 5 = Phrygian
  [0, 1, 3, 5, 6, 8, 10], // 6 = Locrian
];

// Named mode aliases (case-insensitive lookup done by caller).
const MODE_NAMES = {
  lydian: 0, ionian: 1, major: 1, mixolydian: 2, dorian: 3,
  aeolian: 4, minor: 4, phrygian: 5, locrian: 6,
};

const VARS = {
  pot0: [OP_LOAD_POT, 0], pot1: [OP_LOAD_POT, 1],
  pot2: [OP_LOAD_POT, 2],
  ldr: [OP_LOAD_LDR], accel_x: [OP_LOAD_AX], accel_y: [OP_LOAD_AY],
};

class Compiler {
  constructor(src) {
    this.src = src.trim();
    this.pos = 0;
    this.code = [];
  }

  peek() { this.skip(); return this.src[this.pos] || ""; }
  skip() { while (this.pos < this.src.length && this.src[this.pos] === " ") this.pos++; }
  eat(ch) {
    this.skip();
    if (this.src[this.pos] !== ch) throw new Error(`Expected '${ch}' at position ${this.pos}`);
    this.pos++;
  }

  emit(...bytes) {
    for (const b of bytes) this.code.push(b);
  }

  word() {
    this.skip();
    const start = this.pos;
    while (this.pos < this.src.length && /[a-z_0-9]/i.test(this.src[this.pos])) this.pos++;
    return this.src.slice(start, this.pos);
  }

  number() {
    this.skip();
    const start = this.pos;
    while (this.pos < this.src.length && /[0-9]/.test(this.src[this.pos])) this.pos++;
    return parseInt(this.src.slice(start, this.pos), 10);
  }

  expr() { return this.ternary(); }

  ternary() {
    const hadCompare = this.compare();
    this.skip();
    if (this.src[this.pos] === "?") {
      // We already emitted: test_val, threshold (from compare's >)
      // But if there was no >, this is just a truthy check: val > 0 ? ...
      // The compare() already handled the > case and emitted both operands.
      // If no > was found, we need to emit: push 0 (threshold) so IF_GT does val > 0
      if (!hadCompare) {
        this.emit(OP_PUSH, 0);
      }
      this.pos++; // skip '?'
      this.expr(); // then-value
      this.eat(":");
      this.expr(); // else-value
      this.emit(OP_IF_GT);
    } else if (hadCompare) {
      // Bare comparison without ternary, e.g. (a > b)
      // Emit an implicit "? 1 : 0" so the result is 1 if true, 0 if false.
      this.emit(OP_PUSH, 1, OP_PUSH, 0, OP_IF_GT);
    }
  }

  compare() {
    this.additive();
    this.skip();
    if (this.src[this.pos] === ">") {
      this.pos++;
      this.additive();
      return true;
    }
    return false;
  }

  additive() {
    this.multiplicative();
    while (true) {
      this.skip();
      const ch = this.src[this.pos];
      if (ch === "+" || ch === "-") {
        this.pos++;
        this.multiplicative();
        this.emit(ch === "+" ? OP_ADD : OP_SUB);
      } else break;
    }
  }

  multiplicative() {
    this.atom();
    while (true) {
      this.skip();
      const ch = this.src[this.pos];
      if (ch === "*" || ch === "/") {
        this.pos++;
        this.atom();
        this.emit(ch === "*" ? OP_MUL : OP_DIV);
      } else break;
    }
  }

  atom() {
    this.skip();
    const ch = this.src[this.pos];

    // Parenthesized expression
    if (ch === "(") {
      this.pos++;
      this.expr();
      this.eat(")");
      return;
    }

    // Number literal
    if (/[0-9]/.test(ch)) {
      const n = this.number();
      this.emit(OP_PUSH, Math.max(0, Math.min(255, n)));
      return;
    }

    // Word: variable or function call
    const saved = this.pos;
    const w = this.word();
    if (!w) throw new Error(`Unexpected character at position ${this.pos}`);

    // Function call: min(a,b), max(a,b), clamp(val,lo,hi), lerp(a,b,t)
    if ((w === "min" || w === "max" || w === "clamp" || w === "lerp") && this.peek() === "(") {
      this.eat("(");
      this.expr();
      this.eat(",");
      this.expr();
      if (w === "clamp" || w === "lerp") {
        this.eat(",");
        this.expr();
      }
      this.eat(")");
      const ops = { min: OP_MIN, max: OP_MAX, clamp: OP_CLAMP, lerp: OP_LERP };
      this.emit(ops[w]);
      return;
    }

    // scale(root, position) or scale(root, position, mode)
    if (w === "scale" && this.peek() === "(") {
      this.eat("(");
      this.expr();   // root note
      this.eat(",");
      this.expr();   // position (0-127)
      let mode = 0;  // default: Lydian
      if (this.peek() === ",") {
        this.eat(",");
        this.skip();
        // Mode can be a number literal or a name like "lydian", "minor", etc.
        if (/[0-9]/.test(this.src[this.pos])) {
          mode = this.number();
        } else {
          const name = this.word().toLowerCase();
          if (!(name in MODE_NAMES)) throw new Error(`Unknown scale mode: '${name}'`);
          mode = MODE_NAMES[name];
        }
        if (mode < 0 || mode > 6) throw new Error(`Scale mode must be 0-6, got ${mode}`);
      }
      this.eat(")");
      this.emit(OP_SCALE, mode);
      return;
    }

    // Variable
    const ops = VARS[w];
    if (!ops) throw new Error(`Unknown variable: ${w}`);
    this.emit(...ops);
  }

  compile() {
    this.expr();
    this.skip();
    if (this.pos < this.src.length) {
      throw new Error(`Unexpected character at position ${this.pos}: '${this.src[this.pos]}'`);
    }
    if (this.code.length > MAX_EXPR) {
      throw new Error(`Expression too complex (${this.code.length} bytes, max ${MAX_EXPR})`);
    }
    return new Uint8Array(this.code);
  }
}

/**
 * Compile an expression string to bytecode.
 */
export function compileExpr(src) {
  if (!src || !src.trim()) return { code: new Uint8Array(0), error: null };
  try {
    const c = new Compiler(src);
    return { code: c.compile(), error: null };
  } catch (e) {
    return { code: null, error: e.message };
  }
}

export function disassemble(code) {
  const parts = [];
  let i = 0;
  while (i < code.length) {
    switch (code[i]) {
      case OP_PUSH:     i++; parts.push(`push ${code[i]}`); break;
      case OP_LOAD_POT: i++; parts.push(`pot${code[i]}`); break;
      case OP_LOAD_LDR: parts.push("ldr"); break;
      case OP_LOAD_AX:  parts.push("accel_x"); break;
      case OP_LOAD_AY:  parts.push("accel_y"); break;
      case OP_ADD:      parts.push("+"); break;
      case OP_SUB:      parts.push("-"); break;
      case OP_MUL:      parts.push("*"); break;
      case OP_DIV:      parts.push("/"); break;
      case OP_MIN:      parts.push("min"); break;
      case OP_MAX:      parts.push("max"); break;
      case OP_CLAMP:    parts.push("clamp"); break;
      case OP_LERP:     parts.push("lerp"); break;
      case OP_SCALE:    i++; parts.push(`scale(mode=${code[i] ?? '?'})`); break;
      case OP_IF_GT:    parts.push("if>"); break;
      default:          parts.push(`?${code[i].toString(16)}`);
    }
    i++;
  }
  return parts.join(" ");
}
