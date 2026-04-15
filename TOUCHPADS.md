# Touch Pad Improvements

Planned improvements to the capacitive touch sensing implementation, based on
comparison with CircuitPython's `touchio` and research into best practices for
the RP2350 platform.

The current implementation (GPIO charge timing with DWT cycle counter, internal
pull-up, E9-safe polarity, no external components) is fundamentally sound.
These improvements target signal quality, reliability, and Embassy integration.

---

## 1. Release Hysteresis (Highest Priority)

**Problem:** A single threshold for both touch and release causes rapid
touch/release cycling when the signal hovers near the threshold edge.

**Fix:** Add a separate release threshold below the touch threshold.

- Touch fires when `filtered > baseline + margin`
- Release fires when `filtered < baseline + margin * 60 / 100`
- The 60% ratio means the signal must drop to 60% of the touch margin before
  release triggers, creating a dead band that prevents oscillation
- Add a `release_threshold: u32` field to `TouchPad`
- Recompute both thresholds in `update_thresholds()`

If 60% proves too sticky (notes feel slow to release), raise toward 70%. If
oscillation persists, lower toward 50%.

---

## 2. Oversampling with Outlier Rejection

**Problem:** One raw measurement per poll. A single spike (from an interrupt
firing during the DWT busy-wait, bus contention, or noise) can trigger a false
state change.

**Fix:** Take 4 measurements per poll, discard the highest and lowest, average
the middle 2 (median-of-4 / trimmed mean).

- Simple average of `[100, 102, 98, 250]` = 137.5 (wrong)
- Median-of-4 of `[98, 100, 102, 250]` → avg(100, 102) = 101 (correct)
- 4 samples at ~25–100 µs each = ~100–400 µs per pad, still well within budget
- No sorting needed for 4 values: find min/max indices, sum the other two,
  divide by 2

---

## 3. IIR / EMA Filter

**Problem:** No smoothing on touch readings. Measurement-to-measurement jitter
feeds directly into the threshold comparison.

**Fix:** Apply an exponential moving average per pad after oversampling.

```
filtered = α × raw_avg + (1 − α) × filtered_prev
```

- `α = 0.3` — settles in ~3 samples (~17 ms at 200 Hz), fast enough for MIDI
  performance, smooth enough to eliminate residual jitter after the median
- Use fixed-point arithmetic to avoid pulling in float support (multiply by
  77/256 ≈ 0.30 and 179/256 ≈ 0.70, or use a shift-based approximation)
- Add `filtered: u32` field to `TouchPad`
- Initialize `filtered = baseline` during calibration
- Compare `filtered` (not raw) against thresholds

If notes feel sluggish, raise α toward 0.5. If noisy, lower toward 0.2.

---

## 4. Sample-Count Debounce

**Problem:** The current 10 ms time-based debounce is coupled to poll rate. If
the poll rate changes (e.g. due to added oversampling or more pads), the
effective number of agreeing samples changes.

**Fix:** Switch to consecutive-sample-count debounce.

- **Press:** 3 consecutive filtered samples above touch threshold
- **Release:** 4 consecutive filtered samples below release threshold
- Asymmetric because a false release (note stutter) is more disruptive than a
  slightly delayed release
- Replace `stable_since: Instant` with `debounce_count: u8` in `TouchPad`
- Increment toward target on each agreeing poll, reset to 0 on disagreement
- At 200 Hz: press latency = 15 ms, release latency = 20 ms

---

## 5. Increase Calibration to 16 Samples

**Problem:** 8 calibration samples gives ~30% more baseline error than
necessary. The baseline sets the reference for all thresholds — inaccuracy here
shifts both touch and release thresholds for the entire session.

**Fix:** Change calibration from 8 to 16 samples.

- `for _ in 0..16` and `sum / 16`
- Adds ~400 µs to startup per pad — imperceptible
- ~30% reduction in baseline standard error vs 8 samples (σ/√16 vs σ/√8)

---

## 6. Calibration Failure Detection

**Problem:** If a pad times out during calibration (e.g. wrong pin, broken
trace, finger resting on pad at startup), the baseline silently becomes the
timeout value (75,000 cycles) and the pad never triggers. CircuitPython handles
this by raising an error.

**Fix:** After calibrating, check if `baseline >= TIMEOUT_CYCLES`. If so:

- Log a defmt warning with the pin number
- Set `count` to exclude the failed pad (or mark it inactive)
- Optionally: if baseline is abnormally high (not timeout but still several
  standard deviations above the mean of other pads), warn about possible finger-
  on-pad-at-startup

---

## 7. Adaptive Baseline Tracking

**Problem:** Baseline is fixed at startup. Temperature and humidity drift over
time shifts the true untouched reading, causing false triggers or missed
touches. Most noticeable in live performance environments with stage lighting.

**Fix:** Slowly blend the baseline toward the current filtered reading when the
pad is untouched.

```
if !touched:
    baseline += (filtered - baseline) * α_baseline
    recompute thresholds from new baseline
```

- `α_baseline = 0.001` per poll (~0.2/s at 200 Hz) — tracks environmental
  drift over ~30 seconds without reacting to touch events
- **Freeze rule:** never update baseline while the pad is in the touched state
- **Negative drift guard:** if `filtered < baseline - margin`, the environment
  has shifted dramatically — force an immediate recalibration (16 fresh samples)
- **Stuck-touch timeout:** if a pad stays touched for >30 seconds continuously,
  force recalibrate. No musical gesture lasts 30 seconds; something is resting
  on the pad.

This is lower priority than items 1–4 because the current fixed baseline is
adequate in stable environments (studio use). Implement if users report
reliability issues in varying conditions.

---

## 8. Migrate to PIO (Embassy Async Integration)

**Problem:** `measure_touch_async` is async in name only — it busy-waits with
a DWT counting loop that blocks the Embassy executor for up to 500 µs per pad.
With oversampling (improvement 2), this becomes 2 ms per pad. No other task
(USB, serial, accelerometer) runs during the measurement window.

**Fix:** Move the charge/discharge counting into PIO state machines.

### Architecture

- One PIO SM per touch pad, running continuously
- All pads measured **in parallel** in hardware, zero CPU involvement
- Embassy task `await`s on the PIO RX FIFO — true async yield, executor runs
  other tasks freely
- CPU only does threshold comparison and event generation

### PIO Program (E9-safe, jtouch-style)

```
; Drive LOW → switch to input (pull-up charges pad) → count cycles until HIGH
; Repeat for a fixed window, push total cycle count to FIFO
; Lower count = more capacitance = finger present

.program capsense
    ; Y = measurement window (e.g. 2^19 = 524,288 PIO cycles ≈ 4.2 ms at 125 MHz)
    mov isr, null
    set y, 1
    in  y, 1
    in  null, 19
    mov y, isr

    mov x, !null            ; X = 0xFFFFFFFF (cycle counter, counts down)

loop:
    set pindirs, 0          ; Pin → input (pull-up charges pad)
busy:
    jmp pin, charged        ; Pin HIGH? Schmitt trigger crossed ~1.6V
    jmp y--, busy           ; Not yet — decrement window counter
    jmp done                ; Window expired

charged:
    set pindirs, 1          ; Pin → output
    set pins, 0             ; Drive LOW (discharge)
    ; Discharge delay: 8 PIO cycles (64 ns at 125 MHz)
    ; Each jmp also decrements the window counter Y
    jmp y--, d1
    jmp done
d1: jmp y--, d2
    jmp done
d2: jmp y--, d3
    jmp done
d3: jmp y--, d4
    jmp done
d4: jmp y--, d5
    jmp done
d5: jmp y--, d6
    jmp done
d6: jmp y--, d7
    jmp done
d7: jmp y--, d8
    jmp done
d8:
    jmp x--, loop           ; Count this cycle, loop

done:
    mov isr, x
    push block              ; Push result to RX FIFO — wakes Embassy task
```

### Pad Setup

- Configure pad pull-up via the pad control register **once** (PIO cannot
  control pull configuration, only pindirs and pin output value)
- Leave pull-up enabled permanently — when pin is output-LOW, the pull-up
  draws ~66 µA (3.3V / 50 kΩ), negligible, and the output driver overwhelms it
  during discharge
- Enable Schmitt trigger on the pad
- Map SM `set_base` and `jmp_pin` to the touch GPIO

### Reading Results

```rust
async fn read_touch(sm_rx: &mut StateMachineRx<'static, PIO0, 0>) -> u32 {
    let raw = sm_rx.wait_pull().await;  // truly yields to executor
    u32::MAX - raw                       // invert: higher = more capacitance
}
```

### Resource Budget

- Program size: ~24 instructions (fits in the 32-slot instruction memory)
- All 4 SMs in a PIO block share one program instance
- RP2350 has 3 PIO blocks × 4 SMs = 12 SMs total
- Supports up to 12 simultaneous touch pads (covers typical use)
- pico-midi currently uses no PIO — all 12 SMs are available

### Benefits over Current GPIO Approach

| Aspect | Current GPIO | PIO |
|--------|-------------|-----|
| Executor blocking | Up to 500 µs/pad busy-wait | Zero — true async FIFO read |
| Multi-pad timing | Sequential | All pads in parallel |
| Interrupt jitter | DWT count inflated by any ISR | Immune — PIO runs independently |
| Oversampling | N× measurement time × CPU cost | Free — thousands of cycles integrated in hardware, SNR > 100:1 |
| CPU cost per poll | ~100–500 µs per pad | ~10 cycles per pad (FIFO read + math) |

### When to Implement

Not urgent — the GPIO approach works for typical 1–8 pad configurations. The
trigger for PIO migration is:

- Adding oversampling (improvement 2) makes busy-wait duration uncomfortable
- Pad count exceeds ~8 and sequential measurement > 4 ms
- Other tasks (USB, serial) need lower latency guarantees

Consider implementing PIO as a replacement for improvements 2 and 3 — PIO's
inherent integration of thousands of charge/discharge cycles provides better
oversampling and noise rejection than any software filter on single-shot GPIO
readings. With PIO, the IIR filter (improvement 3) becomes optional (still
useful for additional smoothing but less critical).

---

## 9. Lower Default Threshold to 25%

**Problem:** With improvements 1–4 in place (hysteresis, oversampling,
filtering, debounce), the current 33% default threshold is more conservative
than necessary. Each layer handles a different noise component:

| Layer | Handles |
|-------|---------|
| Oversampling | Single-sample spikes |
| IIR filter | High-frequency jitter |
| Hysteresis | Threshold-edge oscillation |
| Debounce | Brief transients |
| Threshold | Gross separation of touched vs untouched |

**Fix:** Lower `threshold_pct` default from 33 to 25 after implementing the
other improvements. This makes lighter touches register — important for
expressive musical performance.

**Do not** lower the threshold without also adding hysteresis and oversampling.
These parameters are a system; change them together.

---

## Implementation Order

1. **Hysteresis** — biggest reliability win, smallest code change
2. **Oversampling** — eliminates spike-induced false triggers
3. **IIR filter** — smooths residual jitter
4. **Sample-count debounce** — more robust than time-based
5. **16-sample calibration** — trivial change, better baseline
6. **Calibration failure detection** — user-facing reliability
7. **Adaptive baseline** — environmental robustness (implement if needed)
8. **PIO migration** — when CPU overhead or pad count demands it
9. **Lower threshold** — only after 1–4 are in place
