# midictrl firmware

Rust firmware for RP2040 and RP2350 dev boards (Pico / Pico 2) that turns them into a USB MIDI controller.

## Features

- USB composite device: MIDI + CDC-ACM serial
- Buttons (GPIO, debounced), potentiometers (ADC), LDR (ADC)
- Capacitive touch pads (charge/discharge sensing)
- Accelerometer (LIS3DH over I2C) — tilt-to-CC and tap detection
- Line-based serial protocol for runtime configuration
- Config stored in flash (last 4K sector, survives power cycles)
- Single crate, dual target via feature flags (`rp2040` / `rp2350`)
- Embassy async framework — no RTOS, no alloc

## Pin assignments (defaults)

| Function       | Pin(s)        |
|----------------|---------------|
| Buttons        | GP2–GP5       |
| Touch pads     | GP6–GP10      |
| Pots           | GP26 (ADC0), GP27 (ADC1) |
| LDR            | GP28 (ADC2)   |
| Accelerometer  | I2C0: SCL=GP1, SDA=GP0   |
| Status LED     | GP25          |

## Build

Requires a nightly or stable Rust toolchain with ARM targets:

```
rustup target add thumbv6m-none-eabi          # RP2040
rustup target add thumbv8m.main-none-eabihf   # RP2350
```

```
make build-2040     # default
make build-2350
make check          # check both targets
```

## Flash

Put the board in BOOTSEL mode, then:

```
make flash-2040
```

For RP2350, copy the UF2 manually:

```
make flash-2350   # prints instructions
```

## Serial protocol

Connect to the CDC-ACM serial port at any baud rate. Commands are line-terminated (`\n`).

| Command         | Response                     |
|-----------------|------------------------------|
| `PING`          | `PONG`                       |
| `VERSION`       | `midictrl 0.1.0`             |
| `GET`           | Hex-encoded config blob      |
| `PUT <hex>`     | `OK` or `ERR bad config`     |
| `SAVE`          | `OK saved` or `ERR save failed` |
| `RESET`         | `OK` (restores defaults in RAM) |
| `REBOOT`        | `OK` (then device resets)    |

### Config format

`GET` returns and `PUT` accepts the same binary format used for flash storage, hex-encoded as a single line. The binary layout is:

```
[magic:4 LE][version:1][midi_channel:1]
[num_buttons:1][buttons: N×3 (pin,note,velocity)]
[num_touch_pads:1][touch_pads: N×3 (pin,note,velocity)]
[num_pots:1][pots: N×2 (pin,cc)]
[ldr_enabled:1][ldr_pin:1][ldr_cc:1]
[accel_enabled:1][sda:1][scl:1][int:1][x_cc:1][y_cc:1][tap_note:1][tap_vel:1][dead_zone:1][smoothing:1]
```

Magic is `0x4D494449` ("MIDI"), version is `1`.

After changing config, send `SAVE` then `REBOOT` to apply.
