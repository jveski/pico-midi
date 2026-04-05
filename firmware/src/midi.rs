//! USB-MIDI message construction.
//!
//! USB-MIDI 1.0 event packets are 4 bytes:
//!   [cable_number << 4 | CIN, status_byte, data1, data2]

/// Code Index Number for Note Off
const CIN_NOTE_OFF: u8 = 0x08;
/// Code Index Number for Note On
const CIN_NOTE_ON: u8 = 0x09;
/// Code Index Number for Control Change
const CIN_CC: u8 = 0x0B;

/// Build a USB-MIDI Note On event packet.
#[inline]
pub fn note_on(channel: u8, note: u8, velocity: u8) -> [u8; 4] {
    [
        CIN_NOTE_ON,
        0x90 | (channel & 0x0F),
        note & 0x7F,
        velocity & 0x7F,
    ]
}

/// Build a USB-MIDI Note Off event packet.
#[inline]
pub fn note_off(channel: u8, note: u8) -> [u8; 4] {
    [
        CIN_NOTE_OFF,
        0x80 | (channel & 0x0F),
        note & 0x7F,
        0,
    ]
}

/// Build a USB-MIDI Control Change event packet.
#[inline]
pub fn control_change(channel: u8, cc: u8, value: u8) -> [u8; 4] {
    [
        CIN_CC,
        0xB0 | (channel & 0x0F),
        cc & 0x7F,
        value & 0x7F,
    ]
}
