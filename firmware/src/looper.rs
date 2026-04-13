//! Live looper engine for layered MIDI loop recording and playback.
//!
//! Supports 2-4 loop layers that play simultaneously, allowing the user
//! to build up layered synth sounds. Events are timestamped in ticks
//! derived from the configured BPM and can be quantized to a grid
//! (1/4, 1/8, or 1/16 notes).
//!
//! The looper runs in the same single-threaded executor as the rest of
//! the firmware, so no synchronization primitives are needed.

use crate::config::{
    LoopConfig, MAX_LOOP_EVENTS, MAX_LOOP_LAYERS, QUANTIZE_EIGHTH, QUANTIZE_QUARTER,
    QUANTIZE_SIXTEENTH,
};

/// Ticks per quarter note (PPQN). Higher values give finer resolution.
/// 96 PPQN is standard for hardware sequencers and gives good resolution
/// at all quantization levels (divisible by 4, 3, 2).
pub const PPQN: u16 = 96;

/// A recorded MIDI event within a loop layer.
#[derive(Clone, Copy)]
pub struct LoopEvent {
    /// Tick position within the loop (0..loop_length_ticks).
    pub tick: u16,
    /// MIDI note number (0-127).
    pub note: u8,
    /// Velocity (0 = note-off, 1-127 = note-on).
    pub velocity: u8,
}

/// State of a single loop layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerState {
    /// Layer is empty, not playing.
    Empty,
    /// Layer is recording (first pass — sets loop length if this is layer 0).
    Recording,
    /// Layer is playing back recorded events.
    Playing,
    /// Layer is muted (events preserved but not played).
    Muted,
}

/// A single loop layer with its event buffer and state.
pub struct LoopLayer {
    pub events: [LoopEvent; MAX_LOOP_EVENTS],
    pub num_events: u16,
    pub state: LayerState,
}

impl LoopLayer {
    const fn new() -> Self {
        Self {
            events: [LoopEvent {
                tick: 0,
                note: 0,
                velocity: 0,
            }; MAX_LOOP_EVENTS],
            num_events: 0,
            state: LayerState::Empty,
        }
    }

    fn clear(&mut self) {
        self.num_events = 0;
        self.state = LayerState::Empty;
    }

    /// Insert an event sorted by tick. Returns false if the buffer is full.
    fn insert_event(&mut self, event: LoopEvent) -> bool {
        let n = self.num_events as usize;
        if n >= MAX_LOOP_EVENTS {
            return false;
        }
        // Find insertion point (maintain sorted order by tick)
        let mut pos = n;
        for i in 0..n {
            if self.events[i].tick > event.tick {
                pos = i;
                break;
            }
        }
        // Shift elements to make room
        if pos < n {
            let mut i = n;
            while i > pos {
                self.events[i] = self.events[i - 1];
                i -= 1;
            }
        }
        self.events[pos] = event;
        self.num_events += 1;
        true
    }
}

/// Action to send from the looper to the MIDI/synth output.
#[derive(Clone, Copy)]
pub struct LoopOutput {
    pub note: u8,
    pub velocity: u8,
}

/// The complete looper state.
pub struct Looper {
    pub layers: [LoopLayer; MAX_LOOP_LAYERS],
    /// Number of active layers (from config, 2-4).
    pub num_layers: u8,
    /// Loop length in ticks. Set from config (bars * beats_per_bar * PPQN).
    pub loop_length_ticks: u16,
    /// Current tick position in the loop (0..loop_length_ticks).
    pub current_tick: u16,
    /// Whether the global transport is running.
    pub playing: bool,
    /// Configured BPM (40-240).
    pub bpm: u8,
    /// Quantization mode.
    pub quantize: u8,
    /// Microseconds per tick, derived from BPM.
    us_per_tick: u32,
    /// Microsecond accumulator for sub-tick timing.
    us_accumulator: u32,
    /// Output buffer for events to emit this tick.
    output_buf: [LoopOutput; 16],
    output_count: u8,
    /// Previous tick (for detecting tick advancement).
    prev_tick: u16,
}

impl Looper {
    pub const fn new() -> Self {
        Self {
            layers: [
                LoopLayer::new(),
                LoopLayer::new(),
                LoopLayer::new(),
                LoopLayer::new(),
            ],
            num_layers: 4,
            loop_length_ticks: 4 * 4 * PPQN, // 4 bars * 4 beats
            current_tick: 0,
            playing: false,
            bpm: 120,
            quantize: QUANTIZE_EIGHTH,
            us_per_tick: 0, // computed in apply_config
            us_accumulator: 0,
            output_buf: [LoopOutput {
                note: 0,
                velocity: 0,
            }; 16],
            output_count: 0,
            prev_tick: 0,
        }
    }

    /// Apply config parameters. Called when config changes.
    pub fn apply_config(&mut self, cfg: &LoopConfig) {
        self.num_layers = cfg.num_layers.clamp(2, MAX_LOOP_LAYERS as u8);
        let bpm = cfg.bpm.clamp(40, 240);
        self.bpm = bpm;
        self.quantize = cfg.quantize.min(QUANTIZE_SIXTEENTH);
        let bars = cfg.bars.clamp(1, 8) as u16;
        let new_length = bars * 4 * PPQN;
        if new_length != self.loop_length_ticks && !self.playing {
            self.loop_length_ticks = new_length;
        }
        // us_per_tick = 60_000_000 / (BPM * PPQN)
        self.us_per_tick = 60_000_000 / (u32::from(bpm) * u32::from(PPQN));
    }

    /// Advance the clock by `elapsed_us` microseconds.
    /// Returns the number of new tick events to process.
    /// Call this every polling loop iteration with the elapsed time.
    pub fn advance(&mut self, elapsed_us: u32) -> u8 {
        if !self.playing || self.us_per_tick == 0 {
            return 0;
        }

        self.us_accumulator += elapsed_us;
        let mut ticks_advanced: u8 = 0;

        // Save the tick *before* advancing so collect_playback_events can
        // replay the full range (prev_tick+1..=current_tick) after the loop.
        self.prev_tick = self.current_tick;

        while self.us_accumulator >= self.us_per_tick {
            self.us_accumulator -= self.us_per_tick;
            self.current_tick += 1;
            if self.current_tick >= self.loop_length_ticks {
                self.current_tick = 0;
            }
            ticks_advanced = ticks_advanced.saturating_add(1);
        }

        ticks_advanced
    }

    /// Collect all loop events that should play for the ticks advanced
    /// since the last call to `advance()`.
    /// This covers every tick in the half-open range (prev_tick, current_tick],
    /// so no events are lost when multiple ticks advance in one polling cycle.
    /// Call this after `advance()` returns > 0.
    /// Returns a slice of `LoopOutput` events to send.
    pub fn collect_playback_events(&mut self) -> &[LoopOutput] {
        self.output_count = 0;
        let start = self.prev_tick;
        let end = self.current_tick;
        let num_layers = self.num_layers as usize;

        for layer in self.layers.iter().take(num_layers) {
            if layer.state != LayerState::Playing {
                continue;
            }
            let n = layer.num_events as usize;
            for i in 0..n {
                let t = layer.events[i].tick;
                let in_range = if end >= start {
                    // Normal case: no wraparound
                    t > start && t <= end
                } else {
                    // Wrapped around the loop boundary
                    t > start || t <= end
                };
                if in_range {
                    let count = self.output_count as usize;
                    if count < self.output_buf.len() {
                        self.output_buf[count] = LoopOutput {
                            note: layer.events[i].note,
                            velocity: layer.events[i].velocity,
                        };
                        self.output_count += 1;
                    }
                }
            }
        }

        &self.output_buf[..self.output_count as usize]
    }

    /// Record a note event into the specified layer.
    /// The event is quantized according to the current quantize setting.
    pub fn record_event(&mut self, layer_idx: u8, note: u8, velocity: u8) {
        let idx = layer_idx as usize;
        if idx >= self.num_layers as usize {
            return;
        }
        if self.layers[idx].state != LayerState::Recording {
            return;
        }

        let tick = self.quantize_tick(self.current_tick);
        self.layers[idx].insert_event(LoopEvent {
            tick,
            note,
            velocity,
        });
    }

    /// Quantize a tick value to the nearest grid point.
    fn quantize_tick(&self, tick: u16) -> u16 {
        let grid = match self.quantize {
            QUANTIZE_QUARTER => PPQN,       // 96 ticks
            QUANTIZE_EIGHTH => PPQN / 2,    // 48 ticks
            QUANTIZE_SIXTEENTH => PPQN / 4, // 24 ticks
            _ => return tick,
        };

        if grid == 0 {
            return tick;
        }

        let remainder = tick % grid;
        if remainder == 0 {
            tick
        } else if remainder <= grid / 2 {
            // Snap backward
            tick - remainder
        } else {
            // Snap forward (may wrap around loop)
            let snapped = tick + (grid - remainder);
            if snapped >= self.loop_length_ticks {
                0
            } else {
                snapped
            }
        }
    }

    /// Start recording on a layer. If no layer is playing yet,
    /// also starts the transport.
    pub fn start_recording(&mut self, layer_idx: u8) {
        let idx = layer_idx as usize;
        if idx >= self.num_layers as usize {
            return;
        }
        // Clear any existing events when starting a fresh recording
        self.layers[idx].clear();
        self.layers[idx].state = LayerState::Recording;

        if !self.playing {
            self.current_tick = 0;
            self.us_accumulator = 0;
            self.prev_tick = 0;
            self.playing = true;
        }
    }

    /// Stop recording on a layer and transition to playing.
    pub fn stop_recording(&mut self, layer_idx: u8) {
        let idx = layer_idx as usize;
        if idx >= self.num_layers as usize {
            return;
        }
        if self.layers[idx].state == LayerState::Recording {
            self.layers[idx].state = LayerState::Playing;
        }
    }

    /// Toggle mute on a layer.
    pub fn toggle_mute(&mut self, layer_idx: u8) {
        let idx = layer_idx as usize;
        if idx >= self.num_layers as usize {
            return;
        }
        match self.layers[idx].state {
            LayerState::Playing => self.layers[idx].state = LayerState::Muted,
            LayerState::Muted => self.layers[idx].state = LayerState::Playing,
            _ => {}
        }
    }

    /// Clear a single layer.
    pub fn clear_layer(&mut self, layer_idx: u8) {
        let idx = layer_idx as usize;
        if idx >= self.num_layers as usize {
            return;
        }
        self.layers[idx].clear();
    }

    /// Stop transport and clear all layers.
    pub fn stop_all(&mut self) {
        self.playing = false;
        self.current_tick = 0;
        self.us_accumulator = 0;
        for layer in &mut self.layers {
            layer.clear();
        }
    }

    /// Stop transport but keep layer data.
    pub fn stop_transport(&mut self) {
        self.playing = false;
        self.us_accumulator = 0;
    }

    /// Start transport from current position.
    pub fn start_transport(&mut self) {
        if !self.playing {
            self.playing = true;
            self.us_accumulator = 0;
        }
    }

    /// Get the index of the first layer that's in Recording state,
    /// or None if no layer is recording.
    pub fn recording_layer(&self) -> Option<u8> {
        for (i, layer) in self
            .layers
            .iter()
            .enumerate()
            .take(self.num_layers as usize)
        {
            if layer.state == LayerState::Recording {
                #[allow(clippy::cast_possible_truncation)]
                return Some(i as u8);
            }
        }
        None
    }

    /// Return the progress through the loop as a value 0-255 (for UI).
    pub fn progress_byte(&self) -> u8 {
        if self.loop_length_ticks == 0 || !self.playing {
            return 0;
        }
        #[allow(clippy::cast_possible_truncation)]
        let p = (u32::from(self.current_tick) * 255 / u32::from(self.loop_length_ticks)) as u8;
        p
    }

    /// Return the number of events in a layer (for UI display).
    pub fn layer_event_count(&self, layer_idx: u8) -> u16 {
        let idx = layer_idx as usize;
        if idx >= MAX_LOOP_LAYERS {
            return 0;
        }
        self.layers[idx].num_events
    }

    /// Return the state of a layer as a u8 for serialization.
    /// 0=Empty, 1=Recording, 2=Playing, 3=Muted
    pub fn layer_state_byte(&self, layer_idx: u8) -> u8 {
        let idx = layer_idx as usize;
        if idx >= MAX_LOOP_LAYERS {
            return 0;
        }
        match self.layers[idx].state {
            LayerState::Empty => 0,
            LayerState::Recording => 1,
            LayerState::Playing => 2,
            LayerState::Muted => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::QUANTIZE_OFF;

    fn make_looper(bpm: u8, bars: u8, quantize: u8) -> Looper {
        let mut looper = Looper::new();
        looper.apply_config(&LoopConfig {
            enabled: true,
            num_layers: 4,
            bpm,
            quantize,
            bars,
        });
        looper
    }

    #[test]
    fn quantize_eighth_note() {
        let looper = make_looper(120, 4, QUANTIZE_EIGHTH);
        // PPQN = 96, eighth = 48 ticks
        // tick 0 -> 0
        assert_eq!(looper.quantize_tick(0), 0);
        // tick 20 -> 0 (closer to 0 than 48)
        assert_eq!(looper.quantize_tick(20), 0);
        // tick 30 -> 48 (closer to 48 than 0)
        assert_eq!(looper.quantize_tick(30), 48);
        // tick 48 -> 48
        assert_eq!(looper.quantize_tick(48), 48);
        // tick 24 -> 0 (exactly halfway snaps backward)
        assert_eq!(looper.quantize_tick(24), 0);
    }

    #[test]
    fn quantize_off_passes_through() {
        let looper = make_looper(120, 4, QUANTIZE_OFF);
        assert_eq!(looper.quantize_tick(17), 17);
        assert_eq!(looper.quantize_tick(95), 95);
    }

    #[test]
    fn quantize_quarter_note() {
        let looper = make_looper(120, 1, QUANTIZE_QUARTER);
        // Quarter = 96 ticks
        assert_eq!(looper.quantize_tick(0), 0);
        assert_eq!(looper.quantize_tick(40), 0);
        assert_eq!(looper.quantize_tick(50), 96);
        assert_eq!(looper.quantize_tick(96), 96);
    }

    #[test]
    fn loop_length_calculation() {
        let looper = make_looper(120, 4, QUANTIZE_OFF);
        // 4 bars * 4 beats * 96 PPQN = 1536 ticks
        assert_eq!(looper.loop_length_ticks, 1536);

        let looper = make_looper(120, 1, QUANTIZE_OFF);
        assert_eq!(looper.loop_length_ticks, 384); // 1 bar * 4 beats * 96
    }

    #[test]
    fn us_per_tick_calculation() {
        let looper = make_looper(120, 4, QUANTIZE_OFF);
        // 60_000_000 / (120 * 96) = 5208 us per tick
        assert_eq!(looper.us_per_tick, 5208);

        let looper = make_looper(60, 4, QUANTIZE_OFF);
        assert_eq!(looper.us_per_tick, 10416);
    }

    #[test]
    fn advance_ticks() {
        let mut looper = make_looper(120, 4, QUANTIZE_OFF);
        looper.playing = true;
        // 5208 us per tick at 120 BPM
        let ticks = looper.advance(5208);
        assert_eq!(ticks, 1);
        assert_eq!(looper.current_tick, 1);

        // Advance by 2 ticks worth
        let ticks = looper.advance(10416);
        assert_eq!(ticks, 2);
        assert_eq!(looper.current_tick, 3);
    }

    #[test]
    fn loop_wraps_around() {
        let mut looper = make_looper(120, 1, QUANTIZE_OFF);
        looper.playing = true;
        // loop_length = 384 ticks
        looper.current_tick = 383;
        looper.us_accumulator = 0;
        let ticks = looper.advance(5208);
        assert_eq!(ticks, 1);
        assert_eq!(looper.current_tick, 0); // wrapped
    }

    #[test]
    fn record_and_playback() {
        let mut looper = make_looper(120, 1, QUANTIZE_OFF);
        looper.start_recording(0);
        assert!(looper.playing);
        assert_eq!(looper.layers[0].state, LayerState::Recording);

        // Record a note-on at tick 0
        looper.record_event(0, 60, 100);
        assert_eq!(looper.layers[0].num_events, 1);

        // Advance to tick 10 and record note-off
        looper.current_tick = 10;
        looper.record_event(0, 60, 0);
        assert_eq!(looper.layers[0].num_events, 2);

        // Stop recording
        looper.stop_recording(0);
        assert_eq!(looper.layers[0].state, LayerState::Playing);

        // Collect events at tick 0 (simulate advancing from last tick to 0)
        looper.prev_tick = looper.loop_length_ticks - 1;
        looper.current_tick = 0;
        let events = looper.collect_playback_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].note, 60);
        assert_eq!(events[0].velocity, 100);
    }

    #[test]
    fn mute_and_unmute() {
        let mut looper = make_looper(120, 1, QUANTIZE_OFF);
        looper.start_recording(0);
        looper.record_event(0, 60, 100);
        looper.stop_recording(0);

        // Mute layer 0
        looper.toggle_mute(0);
        assert_eq!(looper.layers[0].state, LayerState::Muted);

        // Events should not play when muted
        looper.prev_tick = looper.loop_length_ticks - 1;
        looper.current_tick = 0;
        let events = looper.collect_playback_events();
        assert_eq!(events.len(), 0);

        // Unmute
        looper.toggle_mute(0);
        assert_eq!(looper.layers[0].state, LayerState::Playing);

        looper.prev_tick = looper.loop_length_ticks - 1;
        looper.current_tick = 0;
        let events = looper.collect_playback_events();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn clear_layer_removes_events() {
        let mut looper = make_looper(120, 1, QUANTIZE_OFF);
        looper.start_recording(0);
        looper.record_event(0, 60, 100);
        looper.stop_recording(0);
        assert_eq!(looper.layers[0].num_events, 1);

        looper.clear_layer(0);
        assert_eq!(looper.layers[0].num_events, 0);
        assert_eq!(looper.layers[0].state, LayerState::Empty);
    }

    #[test]
    fn events_sorted_by_tick() {
        let mut looper = make_looper(120, 1, QUANTIZE_OFF);
        looper.start_recording(0);

        looper.current_tick = 100;
        looper.record_event(0, 60, 100);

        looper.current_tick = 50;
        looper.record_event(0, 62, 80);

        looper.current_tick = 200;
        looper.record_event(0, 64, 90);

        let layer = &looper.layers[0];
        assert_eq!(layer.num_events, 3);
        assert_eq!(layer.events[0].tick, 50);
        assert_eq!(layer.events[1].tick, 100);
        assert_eq!(layer.events[2].tick, 200);
    }

    #[test]
    fn recording_layer_detection() {
        let mut looper = make_looper(120, 1, QUANTIZE_OFF);
        assert_eq!(looper.recording_layer(), None);

        looper.start_recording(2);
        assert_eq!(looper.recording_layer(), Some(2));

        looper.stop_recording(2);
        assert_eq!(looper.recording_layer(), None);
    }

    #[test]
    fn progress_byte() {
        let mut looper = make_looper(120, 1, QUANTIZE_OFF);
        looper.playing = true;
        // loop_length = 384
        looper.current_tick = 0;
        assert_eq!(looper.progress_byte(), 0);

        looper.current_tick = 192; // halfway
        assert_eq!(looper.progress_byte(), 127); // 192 * 255 / 384 = 127

        looper.current_tick = 383;
        assert_eq!(looper.progress_byte(), 254);
    }

    #[test]
    fn stop_all_clears_everything() {
        let mut looper = make_looper(120, 1, QUANTIZE_OFF);
        looper.start_recording(0);
        looper.record_event(0, 60, 100);
        looper.stop_recording(0);
        looper.start_recording(1);
        looper.record_event(1, 62, 80);

        looper.stop_all();
        assert!(!looper.playing);
        assert_eq!(looper.current_tick, 0);
        for layer in &looper.layers {
            assert_eq!(layer.state, LayerState::Empty);
            assert_eq!(layer.num_events, 0);
        }
    }

    #[test]
    fn quantize_recording() {
        let mut looper = make_looper(120, 1, QUANTIZE_EIGHTH);
        looper.start_recording(0);

        // Record at tick 20, should quantize to 0 (eighth = 48 ticks)
        looper.current_tick = 20;
        looper.record_event(0, 60, 100);

        // Record at tick 30, should quantize to 48
        looper.current_tick = 30;
        looper.record_event(0, 62, 80);

        let layer = &looper.layers[0];
        assert_eq!(layer.num_events, 2);
        assert_eq!(layer.events[0].tick, 0);
        assert_eq!(layer.events[1].tick, 48);
    }

    #[test]
    fn multi_tick_advance_collects_all_events() {
        let mut looper = make_looper(120, 1, QUANTIZE_OFF);
        looper.start_recording(0);

        // Record events on ticks 1, 2, and 3
        looper.current_tick = 1;
        looper.record_event(0, 60, 100);
        looper.current_tick = 2;
        looper.record_event(0, 62, 90);
        looper.current_tick = 3;
        looper.record_event(0, 64, 80);

        looper.stop_recording(0);

        // Simulate advancing 3 ticks at once (prev_tick=0, current_tick=3)
        looper.prev_tick = 0;
        looper.current_tick = 3;
        let events = looper.collect_playback_events();
        assert_eq!(events.len(), 3);

        // Verify all three notes are present
        let notes: Vec<u8> = events.iter().map(|e| e.note).collect();
        assert!(notes.contains(&60));
        assert!(notes.contains(&62));
        assert!(notes.contains(&64));
    }

    #[test]
    fn multi_tick_advance_across_loop_boundary() {
        let mut looper = make_looper(120, 1, QUANTIZE_OFF);
        // loop_length = 384
        looper.start_recording(0);

        // Record at tick 383 (near end) and tick 0 (start)
        looper.current_tick = 383;
        looper.record_event(0, 60, 100);
        looper.current_tick = 0;
        looper.record_event(0, 62, 90);

        looper.stop_recording(0);

        // Simulate wrapping: prev_tick=382, current_tick=0 (advanced 2 ticks)
        looper.prev_tick = 382;
        looper.current_tick = 0;
        let events = looper.collect_playback_events();
        assert_eq!(events.len(), 2);
    }
}
