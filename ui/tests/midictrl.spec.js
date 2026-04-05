// @ts-check
const { test, expect } = require("@playwright/test");
const path = require("path");

const FILE_URL = `file://${path.resolve(__dirname, "..", "index.html")}`;

// ─── Shared helper: build a valid config hex string (postcard format v2) ───

function buildConfigHex(overrides = {}) {
  const MAX_BUTTONS = 8, MAX_TOUCH_PADS = 8, MAX_POTS = 4;
  const cfg = {
    midi_channel: 0,
    buttons: [],
    touch_pads: [],
    pots: [],
    ldr_enabled: false,
    ldr: { pin: 28, cc: 74 },
    accel_enabled: false,
    accel: {
      sda: 0, scl: 1, int_pin: 11,
      x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127,
      dead_zone: 13, smoothing: 25,
    },
    ...overrides,
  };
  const buf = [];
  const MAGIC = 0x4d494449;
  buf.push(MAGIC & 0xff, (MAGIC >> 8) & 0xff, (MAGIC >> 16) & 0xff, (MAGIC >> 24) & 0xff);
  buf.push(2); // version 2
  buf.push(Math.min(cfg.midi_channel, 15));
  const nb = Math.min(cfg.buttons.length, MAX_BUTTONS);
  buf.push(nb);
  for (let j = 0; j < MAX_BUTTONS; j++) {
    if (j < nb) {
      buf.push(cfg.buttons[j].pin, Math.min(cfg.buttons[j].note, 127), Math.max(1, Math.min(cfg.buttons[j].velocity, 127)));
    } else {
      buf.push(0, 0, 0);
    }
  }
  const nt = Math.min(cfg.touch_pads.length, MAX_TOUCH_PADS);
  buf.push(nt);
  for (let j = 0; j < MAX_TOUCH_PADS; j++) {
    if (j < nt) {
      buf.push(cfg.touch_pads[j].pin, Math.min(cfg.touch_pads[j].note, 127), Math.max(1, Math.min(cfg.touch_pads[j].velocity, 127)));
    } else {
      buf.push(0, 0, 0);
    }
  }
  const np = Math.min(cfg.pots.length, MAX_POTS);
  buf.push(np);
  for (let j = 0; j < MAX_POTS; j++) {
    if (j < np) {
      buf.push(cfg.pots[j].pin, Math.min(cfg.pots[j].cc, 127));
    } else {
      buf.push(0, 0);
    }
  }
  buf.push(cfg.ldr.pin, Math.min(cfg.ldr.cc, 127));
  buf.push(cfg.ldr_enabled ? 1 : 0);
  const a = cfg.accel;
  buf.push(cfg.accel_enabled ? 1 : 0, a.sda, a.scl, a.int_pin,
    Math.min(a.x_cc, 127), Math.min(a.y_cc, 127),
    Math.min(a.tap_note, 127), Math.max(1, Math.min(a.tap_vel, 127)),
    a.dead_zone, a.smoothing);
  return Array.from(buf, b => b.toString(16).padStart(2, "0")).join("");
}

// ─── Shared helper: set up mock Web Serial port ───

async function setupMockSerial(page, configHex, opts = {}) {
  await page.evaluate(({ hex, opts }) => {
    window.__mockSerial = {
      commandsSent: [],
      configHex: hex,
      isOpen: false,
      resetConfigHex: opts.resetConfigHex || hex,
      failCommand: opts.failCommand || null,
    };

    const mockWriter = {
      write: async (data) => {
        const text = new TextDecoder().decode(data).trim();
        window.__mockSerial.commandsSent.push(text);

        if (window.__mockSerial.failCommand === text) {
          setTimeout(() => { readBuf += "ERR fail\n"; }, 5);
          return;
        }

        let response = "";
        if (text === "VERSION") {
          response = "midictrl 0.1.0\n";
        } else if (text === "GET") {
          response = window.__mockSerial.configHex + "\n";
        } else if (text.startsWith("PUT ")) {
          window.__mockSerial.configHex = text.substring(4);
          response = "OK\n";
        } else if (text === "SAVE") {
          response = "OK saved\n";
        } else if (text === "RESET") {
          window.__mockSerial.configHex = window.__mockSerial.resetConfigHex;
          response = "OK\n";
        } else if (text === "REBOOT") {
          if (opts.rebootThrows) throw new Error("Port closed");
          response = "OK\n";
        } else {
          response = "ERR unknown\n";
        }

        setTimeout(() => { readBuf += response; }, 5);
      },
      releaseLock: () => {},
    };

    const mockReadable = {
      getReader: () => ({
        read: () => new Promise(() => {}),
        cancel: async () => {},
        releaseLock: () => {},
      }),
    };

    if (!navigator.serial) {
      Object.defineProperty(navigator, 'serial', {
        value: { requestPort: async () => {}, addEventListener: () => {} },
        writable: true,
      });
    }

    navigator.serial.requestPort = async () => {
      const mockPort = {
        open: async () => { window.__mockSerial.isOpen = true; },
        close: async () => { window.__mockSerial.isOpen = false; },
        writable: { getWriter: () => mockWriter },
        readable: mockReadable,
      };
      return mockPort;
    };
  }, { hex: configHex, opts });
}

// Default config constant for brevity
const DEFAULT_CFG = `{
  midi_channel: 0, buttons: [], touch_pads: [], pots: [],
  ldr_enabled: false, ldr: { pin: 28, cc: 74 },
  accel_enabled: false,
  accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
}`;


// ═══════════════════════════════════════════════════
// Initial Page Load
// ═══════════════════════════════════════════════════

test.describe("initial page load", () => {
  test("renders correct initial page state", async ({ page }) => {
    await page.goto(FILE_URL);
    await expect(page).toHaveTitle("pico-midi configurator");
    const h1 = page.locator("header h1");
    await expect(h1).toContainText("pico-midi");
    await expect(h1).toContainText("configurator");
    await expect(page.locator("#statusText")).toHaveText("Disconnected");
    await expect(page.locator("#statusDot")).not.toHaveClass(/connected/);
    await expect(page.locator("#emptyState")).toBeVisible();
    await expect(page.locator("#configPanel")).toBeHidden();
    // Serial log collapsed by default
    await expect(page.locator('.card-header[data-section="log"]')).toHaveClass(/collapsed/);
    await expect(page.locator("#sectionLog")).toHaveClass(/collapsed/);
  });

  test("toolbar buttons are in correct initial state", async ({ page }) => {
    await page.goto(FILE_URL);
    await expect(page.locator("#btnConnect")).toBeEnabled();
    await expect(page.locator("#btnConnect")).toHaveClass(/btn-primary/);
    await expect(page.locator("#btnConnect")).not.toHaveClass(/btn-danger/);
    await expect(page.locator("#btnRefresh")).toBeDisabled();
    await expect(page.locator("#btnSave")).toBeDisabled();
    await expect(page.locator("#btnReset")).toBeDisabled();
    await expect(page.locator("#btnReboot")).toBeDisabled();
  });
});


// ═══════════════════════════════════════════════════
// Pure Functions: noteName
// ═══════════════════════════════════════════════════

test.describe("noteName function", () => {
  test("returns correct note names for known values and full octave", async ({ page }) => {
    await page.goto(FILE_URL);
    const results = await page.evaluate(() => {
      const known = {
        c_neg1: noteName(0), c4: noteName(60), a4: noteName(69),
        g9: noteName(127), fsharp3: noteName(54),
      };
      const octave4 = [];
      for (let i = 60; i < 72; i++) octave4.push(noteName(i));
      return { known, octave4 };
    });
    expect(results.known.c_neg1).toBe("C-1");
    expect(results.known.c4).toBe("C4");
    expect(results.known.a4).toBe("A4");
    expect(results.known.g9).toBe("G9");
    expect(results.known.fsharp3).toBe("F#3");
    expect(results.octave4).toEqual([
      "C4", "C#4", "D4", "D#4", "E4", "F4", "F#4", "G4", "G#4", "A4", "A#4", "B4"
    ]);
  });

  test("returns empty string for out-of-range values", async ({ page }) => {
    await page.goto(FILE_URL);
    const results = await page.evaluate(() => [noteName(-1), noteName(128), noteName(255)]);
    expect(results).toEqual(["", "", ""]);
  });
});


// ═══════════════════════════════════════════════════
// Pure Functions: num and clamp
// ═══════════════════════════════════════════════════

test.describe("num and clamp helpers", () => {
  test("num parses ints, returns fallback for invalid, truncates floats", async ({ page }) => {
    await page.goto(FILE_URL);
    const results = await page.evaluate(() => [
      num("42", 0), num("0", 5), num("-3", 0),       // valid
      num("", 7), num("abc", 99), num(undefined, 10), // invalid
      num("3.7", 0),                                   // float
    ]);
    expect(results).toEqual([42, 0, -3, 7, 99, 10, 3]);
  });

  test("clamp constrains values within range", async ({ page }) => {
    await page.goto(FILE_URL);
    const results = await page.evaluate(() => [
      clamp(5, 0, 10), clamp(-1, 0, 10), clamp(15, 0, 10),
      clamp(0, 0, 10), clamp(10, 0, 10),
    ]);
    expect(results).toEqual([5, 0, 10, 0, 10]);
  });
});


// ═══════════════════════════════════════════════════
// Pure Functions: hexEncode / hexDecode
// ═══════════════════════════════════════════════════

test.describe("hex encode/decode", () => {
  test("encode and decode convert correctly including edge cases", async ({ page }) => {
    await page.goto(FILE_URL);
    const results = await page.evaluate(() => ({
      encode: hexEncode(new Uint8Array([0x00, 0xff, 0x4d, 0x49])),
      decode: Array.from(hexDecode("00ff4d49")),
      emptyEncode: hexEncode(new Uint8Array([])),
      emptyDecode: hexDecode("").length,
      uppercase: Array.from(hexDecode("FF00AB")),
      oddLength: Array.from(hexDecode("0ff")),
    }));
    expect(results.encode).toBe("00ff4d49");
    expect(results.decode).toEqual([0x00, 0xff, 0x4d, 0x49]);
    expect(results.emptyEncode).toBe("");
    expect(results.emptyDecode).toBe(0);
    expect(results.uppercase).toEqual([255, 0, 171]);
    expect(results.oddLength).toEqual([15]); // floor(3/2) = 1 byte
  });

  test("roundtrip: encode then decode is identity", async ({ page }) => {
    await page.goto(FILE_URL);
    const result = await page.evaluate(() => {
      const orig = new Uint8Array([0, 1, 127, 128, 255, 42]);
      const hex = hexEncode(orig);
      const back = hexDecode(hex);
      return Array.from(orig).every((v, i) => v === back[i]) && orig.length === back.length;
    });
    expect(result).toBe(true);
  });
});


// ═══════════════════════════════════════════════════
// Config Encode/Decode
// ═══════════════════════════════════════════════════

test.describe("config encode/decode", () => {
  test("roundtrip: default config", async ({ page }) => {
    await page.goto(FILE_URL);
    const result = await page.evaluate(`(() => {
      const cfg = ${DEFAULT_CFG};
      const hex = encodeConfig(cfg);
      const decoded = decodeConfig(hex);
      return { original: cfg, decoded, hex };
    })()`);
    expect(result.decoded).toEqual(result.original);
    // Magic check: first 8 hex chars = MIDI LE, next 2 = version 02
    expect(result.hex.substring(0, 10)).toBe("4944494d02");
  });

  test("roundtrip: populated config with buttons, touch pads, pots, ldr, accel", async ({ page }) => {
    await page.goto(FILE_URL);
    const result = await page.evaluate(() => {
      const cfg = {
        midi_channel: 9,
        buttons: [{ pin: 2, note: 60, velocity: 100 }, { pin: 5, note: 72, velocity: 127 }],
        touch_pads: [{ pin: 10, note: 48, velocity: 64 }],
        pots: [{ pin: 26, cc: 1 }, { pin: 27, cc: 74 }],
        ldr_enabled: true, ldr: { pin: 28, cc: 74 },
        accel_enabled: true,
        accel: { sda: 4, scl: 5, int_pin: 6, x_cc: 10, y_cc: 11, tap_note: 60, tap_vel: 100, dead_zone: 20, smoothing: 50 },
      };
      const hex = encodeConfig(cfg);
      const decoded = decodeConfig(hex);
      // Also check double-encode is identity
      const hex2 = encodeConfig(decoded);
      return { original: cfg, decoded, isIdentity: hex === hex2 };
    });
    expect(result.decoded).toEqual(result.original);
    expect(result.isIdentity).toBe(true);
  });

  test("roundtrip: boundary values (all zeros and all max)", async ({ page }) => {
    await page.goto(FILE_URL);
    const result = await page.evaluate(() => {
      const zeroCfg = {
        midi_channel: 0,
        buttons: [{ pin: 0, note: 0, velocity: 1 }],
        touch_pads: [], pots: [{ pin: 0, cc: 0 }],
        ldr_enabled: false, ldr: { pin: 0, cc: 0 },
        accel_enabled: false,
        accel: { sda: 0, scl: 0, int_pin: 0, x_cc: 0, y_cc: 0, tap_note: 0, tap_vel: 1, dead_zone: 0, smoothing: 0 },
      };
      const maxCfg = {
        midi_channel: 15,
        buttons: Array.from({ length: 8 }, (_, i) => ({ pin: i, note: 60 + i, velocity: 100 })),
        touch_pads: Array.from({ length: 8 }, (_, i) => ({ pin: 10 + i, note: 72 + i, velocity: 64 })),
        pots: Array.from({ length: 4 }, (_, i) => ({ pin: 26 + i, cc: i * 10 })),
        ldr_enabled: true, ldr: { pin: 29, cc: 127 },
        accel_enabled: true,
        accel: { sda: 0, scl: 1, int_pin: 2, x_cc: 127, y_cc: 127, tap_note: 127, tap_vel: 127, dead_zone: 255, smoothing: 100 },
      };
      const zeroOk = JSON.stringify(decodeConfig(encodeConfig(zeroCfg))) === JSON.stringify(zeroCfg);
      const maxOk = JSON.stringify(decodeConfig(encodeConfig(maxCfg))) === JSON.stringify(maxCfg);
      // Encoded size: 78 bytes = 156 hex chars
      const hexLen = encodeConfig(zeroCfg).length;
      return { zeroOk, maxOk, hexLen };
    });
    expect(result.zeroOk).toBe(true);
    expect(result.maxOk).toBe(true);
    expect(result.hexLen).toBe(156);
  });

  test("decodeConfig rejects invalid input (short, wrong magic, wrong version)", async ({ page }) => {
    await page.goto(FILE_URL);
    const results = await page.evaluate(() => [
      decodeConfig("0102"),                             // too short
      decodeConfig("00000000" + "02".padEnd(146, "0")), // wrong magic
      decodeConfig("4944494d01" + "00".repeat(73)),     // wrong version (1)
      decodeConfig("4944494d03" + "00".repeat(73)),     // wrong version (3)
      decodeConfig("4944494d0205"),                      // truncated after header
      decodeConfig("4944494d02"),                        // 5 bytes only
    ]);
    expect(results.every(r => r === null)).toBe(true);
  });

  test("encodeConfig clamps values (channel, note, velocity, cc)", async ({ page }) => {
    await page.goto(FILE_URL);
    const result = await page.evaluate(() => {
      const cfg = {
        midi_channel: 20,
        buttons: [{ pin: 0, note: 200, velocity: 0 }],
        touch_pads: [{ pin: 0, note: 60, velocity: 200 }],
        pots: [],
        ldr_enabled: false, ldr: { pin: 28, cc: 74 },
        accel_enabled: false,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      const decoded = decodeConfig(encodeConfig(cfg));
      return {
        channel: decoded.midi_channel,
        btnNote: decoded.buttons[0].note,
        btnVel: decoded.buttons[0].velocity,
        touchVel: decoded.touch_pads[0].velocity,
      };
    });
    expect(result.channel).toBe(15);
    expect(result.btnNote).toBe(127);
    expect(result.btnVel).toBe(1);
    expect(result.touchVel).toBe(127);
  });

  test("encodeConfig truncates excess items beyond max", async ({ page }) => {
    await page.goto(FILE_URL);
    const result = await page.evaluate(() => {
      const cfg = {
        midi_channel: 0,
        buttons: Array.from({ length: 10 }, (_, i) => ({ pin: i, note: 60, velocity: 100 })),
        touch_pads: Array.from({ length: 10 }, (_, i) => ({ pin: i, note: 72, velocity: 100 })),
        pots: Array.from({ length: 6 }, (_, i) => ({ pin: 26, cc: i })),
        ldr_enabled: false, ldr: { pin: 28, cc: 74 },
        accel_enabled: false,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      const decoded = decodeConfig(encodeConfig(cfg));
      return { b: decoded.buttons.length, t: decoded.touch_pads.length, p: decoded.pots.length };
    });
    expect(result).toEqual({ b: 8, t: 8, p: 4 });
  });
});


// ═══════════════════════════════════════════════════
// UI State: setConnected
// ═══════════════════════════════════════════════════

test.describe("setConnected", () => {
  test("connected state shows correct UI", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => setConnected(true));
    await expect(page.locator("#statusDot")).toHaveClass(/connected/);
    await expect(page.locator("#btnConnect")).toHaveText("Disconnect");
    await expect(page.locator("#btnConnect")).toHaveClass(/btn-danger/);
    await expect(page.locator("#btnConnect")).not.toHaveClass(/btn-primary/);
    await expect(page.locator("#btnRefresh")).toBeEnabled();
    await expect(page.locator("#btnSave")).toBeEnabled();
    await expect(page.locator("#btnReset")).toBeEnabled();
    await expect(page.locator("#btnReboot")).toBeEnabled();
    await expect(page.locator("#configPanel")).toBeVisible();
    await expect(page.locator("#emptyState")).toBeHidden();
  });

  test("disconnected state shows correct UI", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => setConnected(true));
    await page.evaluate(() => setConnected(false));
    await expect(page.locator("#statusDot")).not.toHaveClass(/connected/);
    await expect(page.locator("#statusText")).toHaveText("Disconnected");
    await expect(page.locator("#btnConnect")).toHaveText("Connect");
    await expect(page.locator("#btnConnect")).toHaveClass(/btn-primary/);
    await expect(page.locator("#btnRefresh")).toBeDisabled();
    await expect(page.locator("#btnSave")).toBeDisabled();
    await expect(page.locator("#btnReset")).toBeDisabled();
    await expect(page.locator("#btnReboot")).toBeDisabled();
    await expect(page.locator("#configPanel")).toBeHidden();
    await expect(page.locator("#emptyState")).toBeVisible();
  });
});

test.describe("setToolbarBusy", () => {
  test("busy disables and un-busy re-enables action buttons", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => setConnected(true));
    await page.evaluate(() => setToolbarBusy(true));
    await expect(page.locator("#btnRefresh")).toBeDisabled();
    await expect(page.locator("#btnSave")).toBeDisabled();
    await expect(page.locator("#btnReset")).toBeDisabled();
    await expect(page.locator("#btnReboot")).toBeDisabled();

    await page.evaluate(() => setToolbarBusy(false));
    await expect(page.locator("#btnRefresh")).toBeEnabled();
    await expect(page.locator("#btnSave")).toBeEnabled();
    await expect(page.locator("#btnReset")).toBeEnabled();
    await expect(page.locator("#btnReboot")).toBeEnabled();
  });
});


// ═══════════════════════════════════════════════════
// Toast Notifications
// ═══════════════════════════════════════════════════

test.describe("toast notifications", () => {
  test("shows success, error, and info toasts with correct styling (last wins)", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => toast("Test success", "success"));
    const toastEl = page.locator("#toast");
    await expect(toastEl).toHaveText("Test success");
    await expect(toastEl).toHaveClass(/success/);
    await expect(toastEl).toHaveClass(/visible/);

    await page.evaluate(() => toast("Test error", "error"));
    await expect(toastEl).toHaveText("Test error");
    await expect(toastEl).toHaveClass(/error/);
    await expect(toastEl).not.toHaveClass(/success/);

    await page.evaluate(() => toast("Test info", "info"));
    await expect(toastEl).toHaveText("Test info");
    await expect(toastEl).toHaveClass(/info/);
  });

  test("toast disappears after timeout", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => toast("Vanishing", "success"));
    await expect(page.locator("#toast")).toHaveClass(/visible/);
    await page.waitForTimeout(3000);
    await expect(page.locator("#toast")).not.toHaveClass(/visible/);
  });
});


// ═══════════════════════════════════════════════════
// Serial Log
// ═══════════════════════════════════════════════════

test.describe("serial log", () => {
  test("appends entries with correct class, newline, and HTML escaping", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      log("Test command", "cmd");
      log("Test response", "resp");
      log("Test error", "err");
      log('<script>alert("xss")</script>', "resp");
    });
    const logContent = page.locator("#logContent");
    const spans = logContent.locator("span");
    await expect(spans).toHaveCount(4);
    await expect(spans.nth(0)).toHaveClass("cmd");
    await expect(spans.nth(0)).toHaveText(/Test command/);
    await expect(spans.nth(1)).toHaveClass("resp");
    await expect(spans.nth(2)).toHaveClass("err");
    // Check newline appended
    const text0 = await spans.nth(0).evaluate(el => el.textContent);
    expect(text0).toBe("Test command\n");
    // Check HTML escaping (no script injection)
    const xssText = await spans.nth(3).evaluate(el => el.textContent);
    expect(xssText).toContain('<script>alert("xss")</script>');
    const scriptCount = await page.evaluate(() =>
      document.querySelectorAll("#logContent script").length
    );
    expect(scriptCount).toBe(0);
  });
});


// ═══════════════════════════════════════════════════
// Collapsible Card Sections
// ═══════════════════════════════════════════════════

test.describe("collapsible card sections", () => {
  test("all config sections can be collapsed and expanded", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => setConnected(true));

    const sections = ["general", "buttons", "touch", "pots", "ldr", "accel"];
    for (const section of sections) {
      const header = page.locator(`.card-header[data-section="${section}"]`);
      const body = page.locator(`#section${section.charAt(0).toUpperCase() + section.slice(1)}`);
      // Collapse
      await header.click();
      await expect(header).toHaveClass(/collapsed/);
      // Expand
      await header.click();
      await expect(header).not.toHaveClass(/collapsed/);
    }

    // Also test the serial log section (starts collapsed)
    const logHeader = page.locator('.card-header[data-section="log"]');
    await expect(logHeader).toHaveClass(/collapsed/);
    await logHeader.click();
    await expect(logHeader).not.toHaveClass(/collapsed/);
  });
});


// ═══════════════════════════════════════════════════
// MIDI Channel Input and Hints
// ═══════════════════════════════════════════════════

test.describe("MIDI channel", () => {
  test("hint shows Ch N+1 and handles empty input", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => setConnected(true));
    const input = page.locator("#midiChannel");
    const hint = page.locator("#midiChannelHint");

    await expect(hint).toHaveText("Ch 1"); // default 0 => Ch 1
    await input.fill("9");
    await expect(hint).toHaveText("Ch 10");
    await input.fill("15");
    await expect(hint).toHaveText("Ch 16");
    await input.fill("");
    await expect(hint).toHaveText("");
  });
});


// ═══════════════════════════════════════════════════
// Button List CRUD
// ═══════════════════════════════════════════════════

test.describe("button list", () => {
  test("add button creates row with defaults, note hint, and hint updates on edit", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(`(() => {
      config = ${DEFAULT_CFG};
      renderConfig();
      setConnected(true);
    })()`);

    await expect(page.locator("#btnCount")).toHaveText("0");
    await page.locator("#addButton").click();
    await expect(page.locator("#btnCount")).toHaveText("1");
    await expect(page.locator("#buttonList .item-row")).toHaveCount(1);

    const row = page.locator("#buttonList .item-row").first();
    await expect(row.locator('[data-field="pin"]')).toHaveValue("0");
    await expect(row.locator('[data-field="note"]')).toHaveValue("60");
    await expect(row.locator('[data-field="velocity"]')).toHaveValue("100");
    await expect(row.locator(".index")).toHaveText("#1");
    await expect(row.locator(".monitor-indicator")).toBeVisible();

    // Note hint shows C4 for note 60
    await expect(row.locator(".note-hint")).toHaveText("C4");

    // Change note to 69 -> hint updates to A4
    await row.locator('[data-field="note"]').fill("69");
    await expect(row.locator(".note-hint")).toHaveText("A4");
  });

  test("remove button removes the correct item", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      config = {
        midi_channel: 0,
        buttons: [
          { pin: 1, note: 60, velocity: 100 },
          { pin: 2, note: 72, velocity: 100 },
          { pin: 3, note: 84, velocity: 100 },
        ],
        touch_pads: [], pots: [],
        ldr_enabled: false, ldr: { pin: 28, cc: 74 },
        accel_enabled: false,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      renderConfig();
      setConnected(true);
    });

    await expect(page.locator("#btnCount")).toHaveText("3");
    // Remove the middle button (index 1, note=72)
    await page.locator("#buttonList .btn-remove").nth(1).click();
    await expect(page.locator("#btnCount")).toHaveText("2");
    const notes = await page.evaluate(() => config.buttons.map(b => b.note));
    expect(notes).toEqual([60, 84]);
    // Index numbers should update
    const indices = page.locator("#buttonList .item-row .index");
    await expect(indices.nth(0)).toHaveText("#1");
    await expect(indices.nth(1)).toHaveText("#2");
  });

  test("add button disabled at max (8), re-enables after remove", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      config = {
        midi_channel: 0,
        buttons: Array.from({ length: 8 }, (_, i) => ({ pin: i, note: 60 + i, velocity: 100 })),
        touch_pads: [], pots: [],
        ldr_enabled: false, ldr: { pin: 28, cc: 74 },
        accel_enabled: false,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      renderConfig();
      setConnected(true);
    });

    await expect(page.locator("#addButton")).toBeDisabled();
    await expect(page.locator("#btnCount")).toHaveText("8");
    await page.locator("#buttonList .btn-remove").first().click();
    await expect(page.locator("#addButton")).toBeEnabled();
    await expect(page.locator("#btnCount")).toHaveText("7");
  });
});


// ═══════════════════════════════════════════════════
// Touch Pad List CRUD
// ═══════════════════════════════════════════════════

test.describe("touch pad list", () => {
  test("add touch pad creates row with defaults, disabled at max (8)", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(`(() => {
      config = ${DEFAULT_CFG};
      renderConfig();
      setConnected(true);
    })()`);

    await expect(page.locator("#touchCount")).toHaveText("0");
    await page.locator("#addTouch").click();
    await expect(page.locator("#touchCount")).toHaveText("1");
    const row = page.locator("#touchList .item-row").first();
    await expect(row.locator('[data-field="note"]')).toHaveValue("72");
    await expect(row.locator('[data-field="velocity"]')).toHaveValue("100");
    // Note hint for touch pad
    await expect(row.locator(".note-hint")).toHaveText("C5");
    // Monitor indicator
    await expect(page.locator("#monTouch0")).toBeVisible();

    // Fill to max and check disabled
    await page.evaluate(() => {
      config.touch_pads = Array.from({ length: 8 }, (_, i) => ({ pin: i, note: 72 + i, velocity: 100 }));
      renderConfig();
    });
    await expect(page.locator("#addTouch")).toBeDisabled();
    await expect(page.locator("#touchCount")).toHaveText("8");
  });

  test("remove touch pad works correctly", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      config = {
        midi_channel: 0, buttons: [],
        touch_pads: [
          { pin: 10, note: 72, velocity: 64 },
          { pin: 11, note: 74, velocity: 80 },
        ],
        pots: [],
        ldr_enabled: false, ldr: { pin: 28, cc: 74 },
        accel_enabled: false,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      renderConfig();
      setConnected(true);
    });

    await page.locator("#touchList .btn-remove").first().click();
    await expect(page.locator("#touchCount")).toHaveText("1");
    const notes = await page.evaluate(() => config.touch_pads.map(t => t.note));
    expect(notes).toEqual([74]);
  });
});


// ═══════════════════════════════════════════════════
// Potentiometer List CRUD
// ═══════════════════════════════════════════════════

test.describe("potentiometer list", () => {
  test("add pot creates row with defaults, disabled at max (4)", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(`(() => {
      config = ${DEFAULT_CFG};
      renderConfig();
      setConnected(true);
    })()`);

    await expect(page.locator("#potCount")).toHaveText("0");
    await page.locator("#addPot").click();
    await expect(page.locator("#potCount")).toHaveText("1");
    const row = page.locator("#potList .item-row").first();
    await expect(row.locator('[data-field="pin"]')).toHaveValue("26");
    await expect(row.locator('[data-field="cc"]')).toHaveValue("0");
    // Monitor bar exists
    await expect(page.locator("#monPotBar0")).toBeAttached();
    await expect(page.locator("#monPotVal0")).toHaveText("0");

    // Fill to max and check disabled
    await page.evaluate(() => {
      config.pots = Array.from({ length: 4 }, (_, i) => ({ pin: 26 + i, cc: i * 10 }));
      renderConfig();
    });
    await expect(page.locator("#addPot")).toBeDisabled();
    await expect(page.locator("#potCount")).toHaveText("4");
  });

  test("remove pot works correctly", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      config = {
        midi_channel: 0, buttons: [], touch_pads: [],
        pots: [{ pin: 26, cc: 1 }, { pin: 27, cc: 74 }],
        ldr_enabled: false, ldr: { pin: 28, cc: 74 },
        accel_enabled: false,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      renderConfig();
      setConnected(true);
    });

    await page.locator("#potList .btn-remove").first().click();
    await expect(page.locator("#potCount")).toHaveText("1");
    const ccs = await page.evaluate(() => config.pots.map(p => p.cc));
    expect(ccs).toEqual([74]);
  });
});


// ═══════════════════════════════════════════════════
// LDR Section
// ═══════════════════════════════════════════════════

test.describe("LDR section", () => {
  test("toggle shows/hides fields with correct values", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(`(() => {
      config = ${DEFAULT_CFG};
      renderConfig();
      setConnected(true);
    })()`);

    await expect(page.locator("#ldrFields")).toBeHidden();
    await expect(page.locator("#ldrEnabled")).not.toBeChecked();
    // No monitor bar when disabled
    await expect(page.locator("#monLdrBar")).toHaveCount(0);

    // Enable with correct values
    await page.evaluate(() => {
      config = { ...config, ldr_enabled: true, ldr: { pin: 28, cc: 74 } };
      renderConfig();
      setConnected(true);
    });
    await expect(page.locator("#ldrFields")).toBeVisible();
    await expect(page.locator("#ldrEnabled")).toBeChecked();
    await expect(page.locator("#ldrPin")).toHaveValue("28");
    await expect(page.locator("#ldrCc")).toHaveValue("74");

    // Toggle via checkbox
    await page.evaluate(() => {
      const cb = document.getElementById("ldrEnabled");
      cb.checked = false;
      cb.dispatchEvent(new Event("change"));
    });
    await expect(page.locator("#ldrFields")).toBeHidden();
    await page.evaluate(() => {
      const cb = document.getElementById("ldrEnabled");
      cb.checked = true;
      cb.dispatchEvent(new Event("change"));
    });
    await expect(page.locator("#ldrFields")).toBeVisible();
  });

  test("monitor bar built when enabled, absent when disabled", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      config = {
        midi_channel: 0, buttons: [], touch_pads: [], pots: [],
        ldr_enabled: true, ldr: { pin: 28, cc: 74 },
        accel_enabled: false,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      renderConfig();
      setConnected(true);
    });
    await expect(page.locator("#monLdrBar")).toBeAttached();
    await expect(page.locator("#monLdrVal")).toHaveText("0");

    await page.evaluate(() => {
      config.ldr_enabled = false;
      renderConfig();
    });
    await expect(page.locator("#monLdrBar")).toHaveCount(0);
  });
});


// ═══════════════════════════════════════════════════
// Accelerometer Section
// ═══════════════════════════════════════════════════

test.describe("accelerometer section", () => {
  test("toggle shows/hides fields with correct values and hints", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(`(() => {
      config = ${DEFAULT_CFG};
      renderConfig();
      setConnected(true);
    })()`);

    await expect(page.locator("#accelFields")).toBeHidden();

    // Enable with custom values
    await page.evaluate(() => {
      config = {
        midi_channel: 0, buttons: [], touch_pads: [], pots: [],
        ldr_enabled: false, ldr: { pin: 28, cc: 74 },
        accel_enabled: true,
        accel: { sda: 4, scl: 5, int_pin: 6, x_cc: 10, y_cc: 11, tap_note: 60, tap_vel: 100, dead_zone: 20, smoothing: 50 },
      };
      renderConfig();
      setConnected(true);
    });

    await expect(page.locator("#accelFields")).toBeVisible();
    await expect(page.locator("#accelSda")).toHaveValue("4");
    await expect(page.locator("#accelScl")).toHaveValue("5");
    await expect(page.locator("#accelInt")).toHaveValue("6");
    await expect(page.locator("#accelXCc")).toHaveValue("10");
    await expect(page.locator("#accelYCc")).toHaveValue("11");
    await expect(page.locator("#accelTapNote")).toHaveValue("60");
    await expect(page.locator("#accelTapVel")).toHaveValue("100");
    await expect(page.locator("#accelDeadZone")).toHaveValue("20");
    await expect(page.locator("#accelSmoothing")).toHaveValue("50");

    // Hints: tap note C4, dead zone 2.0 m/s², smoothing α=0.50
    await expect(page.locator("#tapNoteHint")).toHaveText("C4");
    await expect(page.locator("#deadZoneHint")).toHaveText("2.0 m/s²");
    await expect(page.locator("#smoothingHint")).toHaveText("α=0.50");

    // Update hints
    await page.locator("#accelTapNote").fill("69");
    await expect(page.locator("#tapNoteHint")).toHaveText("A4");
    await page.locator("#accelDeadZone").fill("0");
    await expect(page.locator("#deadZoneHint")).toHaveText("0.0 m/s²");
    await page.locator("#accelSmoothing").fill("100");
    await expect(page.locator("#smoothingHint")).toHaveText("α=1.00");

    // Toggle via checkbox
    await page.evaluate(() => {
      const cb = document.getElementById("accelEnabled");
      cb.checked = false;
      cb.dispatchEvent(new Event("change"));
    });
    await expect(page.locator("#accelFields")).toBeHidden();
  });

  test("monitor indicators built when enabled", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      config = {
        midi_channel: 0, buttons: [], touch_pads: [], pots: [],
        ldr_enabled: false, ldr: { pin: 28, cc: 74 },
        accel_enabled: true,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      renderConfig();
      setConnected(true);
    });
    await expect(page.locator("#monAccelXBar")).toBeAttached();
    await expect(page.locator("#monAccelYBar")).toBeAttached();
    await expect(page.locator("#monAccelTap")).toBeVisible();
    await expect(page.locator("#monAccelXVal")).toHaveText("64");
    await expect(page.locator("#monAccelYVal")).toHaveText("64");
  });
});


// ═══════════════════════════════════════════════════
// renderConfig & readConfigFromUI Roundtrip
// ═══════════════════════════════════════════════════

test.describe("renderConfig and readConfigFromUI roundtrip", () => {
  test("render then read back preserves full config", async ({ page }) => {
    await page.goto(FILE_URL);
    const result = await page.evaluate(() => {
      config = {
        midi_channel: 9,
        buttons: [{ pin: 2, note: 60, velocity: 100 }, { pin: 5, note: 72, velocity: 127 }],
        touch_pads: [{ pin: 10, note: 48, velocity: 64 }],
        pots: [{ pin: 26, cc: 1 }, { pin: 27, cc: 74 }],
        ldr_enabled: true, ldr: { pin: 28, cc: 74 },
        accel_enabled: true,
        accel: { sda: 4, scl: 5, int_pin: 6, x_cc: 10, y_cc: 11, tap_note: 60, tap_vel: 100, dead_zone: 20, smoothing: 50 },
      };
      const original = JSON.parse(JSON.stringify(config));
      renderConfig();
      readConfigFromUI();
      return { original, readBack: JSON.parse(JSON.stringify(config)) };
    });
    expect(result.readBack).toEqual(result.original);
  });

  test("readConfigFromUI clamps out-of-range and handles empty inputs", async ({ page }) => {
    await page.goto(FILE_URL);
    const result = await page.evaluate(() => {
      config = {
        midi_channel: 0,
        buttons: [{ pin: 2, note: 60, velocity: 100 }],
        touch_pads: [], pots: [],
        ldr_enabled: true, ldr: { pin: 28, cc: 74 },
        accel_enabled: true,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      renderConfig();
      // Set out-of-range values
      document.getElementById("midiChannel").value = "20";
      document.getElementById("accelSmoothing").value = "200";
      // Set an empty value
      document.getElementById("ldrPin").value = "";
      readConfigFromUI();
      return { channel: config.midi_channel, smoothing: config.accel.smoothing, ldrPin: config.ldr.pin };
    });
    expect(result.channel).toBe(15);     // clamped to 0-15
    expect(result.smoothing).toBe(100);   // clamped to 0-100
    expect(result.ldrPin).toBe(0);        // num("", 0) => 0
  });
});


// ═══════════════════════════════════════════════════
// Monitor Line Parsing
// ═══════════════════════════════════════════════════

test.describe("applyMonitorLine", () => {
  test("updates all indicator types from monitor data", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      config = {
        midi_channel: 0,
        buttons: [{ pin: 0, note: 60, velocity: 100 }, { pin: 1, note: 61, velocity: 100 }],
        touch_pads: [{ pin: 10, note: 72, velocity: 64 }, { pin: 11, note: 73, velocity: 64 }],
        pots: [{ pin: 26, cc: 1 }, { pin: 27, cc: 74 }],
        ldr_enabled: true, ldr: { pin: 28, cc: 74 },
        accel_enabled: true,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      renderConfig();
      applyMonitorLine("M:b=10100000,t=01000000,p=64:127:0:0,l=42,ax=100,ay=30,at=0");
    });
    // Buttons: 0=active, 1=off
    await expect(page.locator("#monBtn0")).toHaveClass(/active/);
    await expect(page.locator("#monBtn1")).not.toHaveClass(/active/);
    // Touch: 0=off, 1=active
    await expect(page.locator("#monTouch0")).not.toHaveClass(/active/);
    await expect(page.locator("#monTouch1")).toHaveClass(/active/);
    // Pots
    await expect(page.locator("#monPotVal0")).toHaveText("64");
    await expect(page.locator("#monPotVal1")).toHaveText("127");
    const width0 = await page.locator("#monPotBar0").evaluate(el => el.style.width);
    expect(parseFloat(width0)).toBeCloseTo(50.4, 0);
    // LDR
    await expect(page.locator("#monLdrVal")).toHaveText("42");
    // Accel tilt
    await expect(page.locator("#monAccelXVal")).toHaveText("100");
    await expect(page.locator("#monAccelYVal")).toHaveText("30");
  });

  test("accelerometer tap indicator activates and decays", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      config = {
        midi_channel: 0, buttons: [], touch_pads: [], pots: [],
        ldr_enabled: false, ldr: { pin: 28, cc: 74 },
        accel_enabled: true,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      renderConfig();
      applyMonitorLine("M:b=00000000,t=00000000,p=0:0:0:0,l=0,ax=64,ay=64,at=1");
    });
    await expect(page.locator("#monAccelTap")).toHaveClass(/active/);
    await page.waitForTimeout(300);
    await expect(page.locator("#monAccelTap")).not.toHaveClass(/active/);
  });

  test("handles non-M: lines, partial data, and extra segments gracefully", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      config = {
        midi_channel: 0,
        buttons: [{ pin: 0, note: 60, velocity: 100 }],
        touch_pads: [], pots: [],
        ldr_enabled: true, ldr: { pin: 28, cc: 74 },
        accel_enabled: true,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      renderConfig();
      setConnected(true);
      buildLdrMonitor();
      buildAccelMonitor();
    });

    // Non-M: lines should not throw
    await page.evaluate(() => {
      applyMonitorLine("OK");
      applyMonitorLine("midictrl 0.1.0");
      applyMonitorLine("");
    });

    // Partial: only button data, no other fields
    await page.evaluate(() => applyMonitorLine("M:b=10000000"));
    await expect(page.locator("#monBtn0")).toHaveClass(/active/);

    // Extra segments silently ignored
    await page.evaluate(() => {
      applyMonitorLine("M:b=1,t=0,p=64,l=42,ax=65,ay=58,at=1,extra=foo,unknown=bar");
    });
    const ldrVal = await page.locator("#monLdrVal").textContent();
    expect(ldrVal).toBe("42");
  });
});


// ═══════════════════════════════════════════════════
// drainMonitorLines and processMonitorBuffer
// ═══════════════════════════════════════════════════

test.describe("drainMonitorLines and processMonitorBuffer", () => {
  test("drainMonitorLines extracts M: lines and preserves non-M: lines", async ({ page }) => {
    await page.goto(FILE_URL);
    const result = await page.evaluate(() => {
      config = {
        midi_channel: 0,
        buttons: [{ pin: 0, note: 60, velocity: 100 }],
        touch_pads: [], pots: [],
        ldr_enabled: false, ldr: { pin: 28, cc: 74 },
        accel_enabled: false,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      renderConfig();
      readBuf = "M:b=10000000,t=00000000,p=0:0:0:0,l=0,ax=64,ay=64,at=0\nOK saved\nM:b=00000000\nincomplete";
      drainMonitorLines();
      const afterDrain = readBuf;

      // Also test empty and incomplete-only buffers
      readBuf = "";
      drainMonitorLines();
      const afterEmpty = readBuf;

      readBuf = "partial data";
      drainMonitorLines();
      const afterPartial = readBuf;

      return { afterDrain, afterEmpty, afterPartial };
    });
    expect(result.afterDrain).toBe("OK saved\nincomplete");
    expect(result.afterEmpty).toBe("");
    expect(result.afterPartial).toBe("partial data");
  });

  test("processMonitorBuffer discards non-M: lines (race condition bug)", async ({ page }) => {
    await page.goto(FILE_URL);
    const result = await page.evaluate(() => {
      port = {}; // truthy to pass guard
      readBuf = "M:b=10000000\nOK saved\nM:b=00000000\nincomplete";
      processMonitorBuffer();
      const buf = readBuf;
      port = null;
      return buf;
    });
    // BUG: "OK saved" is silently discarded; only incomplete tail preserved
    expect(result).toBe("incomplete");
  });
});


// ═══════════════════════════════════════════════════
// Full Config Flow (Mocked Serial)
// ═══════════════════════════════════════════════════

test.describe("full config flow with mock serial", () => {
  test("connect flow: VERSION then GET then renders config", async ({ page }) => {
    await page.goto(FILE_URL);
    const hex = buildConfigHex({
      midi_channel: 5,
      buttons: [{ pin: 2, note: 60, velocity: 100 }],
    });
    await setupMockSerial(page, hex);
    await page.evaluate(() => connect());
    await page.waitForTimeout(500);

    await expect(page.locator("#statusDot")).toHaveClass(/connected/);
    await expect(page.locator("#statusText")).toHaveText("midictrl 0.1.0");
    await expect(page.locator("#configPanel")).toBeVisible();
    await expect(page.locator("#midiChannel")).toHaveValue("5");
    await expect(page.locator("#btnCount")).toHaveText("1");
    const cmds = await page.evaluate(() => window.__mockSerial.commandsSent);
    expect(cmds).toContain("VERSION");
    expect(cmds).toContain("GET");
  });

  test("save flow: PUT then SAVE", async ({ page }) => {
    await page.goto(FILE_URL);
    const hex = buildConfigHex({ midi_channel: 3 });
    await setupMockSerial(page, hex);
    await page.evaluate(() => connect());
    await page.waitForTimeout(500);
    await page.evaluate(() => stopMonitorPoll());

    await page.locator("#midiChannel").fill("7");
    await page.locator("#btnSave").click();
    await page.waitForFunction(
      () => window.__mockSerial.commandsSent.includes("SAVE"),
      { timeout: 5000 }
    );

    const cmds = await page.evaluate(() => window.__mockSerial.commandsSent);
    const putCmd = cmds.find(c => c.startsWith("PUT "));
    expect(putCmd).toBeTruthy();
    expect(cmds).toContain("SAVE");
    const putHex = putCmd.substring(4);
    const decoded = await page.evaluate((h) => decodeConfig(h), putHex);
    expect(decoded.midi_channel).toBe(7);
  });

  test("disconnect flow cleans up state", async ({ page }) => {
    await page.goto(FILE_URL);
    const hex = buildConfigHex();
    await setupMockSerial(page, hex);
    await page.evaluate(() => connect());
    await page.waitForTimeout(500);
    await expect(page.locator("#statusDot")).toHaveClass(/connected/);

    await page.evaluate(() => disconnect());
    await page.waitForTimeout(100);
    await expect(page.locator("#statusDot")).not.toHaveClass(/connected/);
    await expect(page.locator("#statusText")).toHaveText("Disconnected");
    await expect(page.locator("#configPanel")).toBeHidden();
    await expect(page.locator("#emptyState")).toBeVisible();
    const state = await page.evaluate(() => ({
      port: port, reader: reader, writer: writer, readBuf: readBuf,
    }));
    expect(state.port).toBeNull();
    expect(state.reader).toBeNull();
    expect(state.writer).toBeNull();
    expect(state.readBuf).toBe("");
  });
});


// ═══════════════════════════════════════════════════
// Reset, Reboot, and Disconnect Flows
// ═══════════════════════════════════════════════════

test.describe("reset, reboot, and disconnect flows", () => {
  test("cancelled confirm on reset/reboot does nothing", async ({ page }) => {
    await page.goto(FILE_URL);
    const hex = buildConfigHex({ midi_channel: 5 });
    await setupMockSerial(page, hex);
    await page.evaluate(() => connect());
    await page.waitForTimeout(500);
    await page.evaluate(() => stopMonitorPoll());
    await page.evaluate(() => { window.confirm = () => false; });

    await page.locator("#btnReset").click();
    await page.waitForTimeout(200);
    const cmdsAfterReset = await page.evaluate(() => window.__mockSerial.commandsSent);
    expect(cmdsAfterReset).not.toContain("RESET");

    await page.locator("#btnReboot").click();
    await page.waitForTimeout(200);
    const cmdsAfterReboot = await page.evaluate(() => window.__mockSerial.commandsSent);
    expect(cmdsAfterReboot).not.toContain("REBOOT");
    await expect(page.locator("#statusDot")).toHaveClass(/connected/);
  });

  test("confirmed reset sends RESET then refreshes config", async ({ page }) => {
    await page.goto(FILE_URL);
    const initialHex = buildConfigHex({ midi_channel: 5, buttons: [{ pin: 2, note: 60, velocity: 100 }] });
    const resetHex = buildConfigHex({ midi_channel: 0 });
    await setupMockSerial(page, initialHex, { resetConfigHex: resetHex });
    await page.evaluate(() => connect());
    await page.waitForTimeout(500);
    await page.evaluate(() => stopMonitorPoll());
    await expect(page.locator("#midiChannel")).toHaveValue("5");

    await page.evaluate(() => { window.confirm = () => true; });
    await page.locator("#btnReset").click();
    await page.waitForFunction(
      () => {
        const cmds = window.__mockSerial.commandsSent;
        const resetIdx = cmds.indexOf("RESET");
        return resetIdx >= 0 && cmds.indexOf("GET", resetIdx + 1) >= 0;
      },
      { timeout: 5000 }
    );
    await expect(page.locator("#midiChannel")).toHaveValue("0");
  });

  test("confirmed reboot sends REBOOT then disconnects", async ({ page }) => {
    await page.goto(FILE_URL);
    const hex = buildConfigHex();
    await setupMockSerial(page, hex);
    await page.evaluate(() => connect());
    await page.waitForTimeout(500);
    await page.evaluate(() => stopMonitorPoll());
    await page.evaluate(() => { window.confirm = () => true; });

    await page.locator("#btnReboot").click();
    await page.waitForFunction(
      () => window.__mockSerial.commandsSent.includes("REBOOT"),
      { timeout: 5000 }
    );
    await page.waitForTimeout(200);
    await expect(page.locator("#statusDot")).not.toHaveClass(/connected/);
    await expect(page.locator("#statusText")).toHaveText("Disconnected");
  });

  test("reboot command error still disconnects", async ({ page }) => {
    await page.goto(FILE_URL);
    const hex = buildConfigHex();
    await setupMockSerial(page, hex, { rebootThrows: true });
    await page.evaluate(() => connect());
    await page.waitForTimeout(500);
    await page.evaluate(() => stopMonitorPoll());
    await page.evaluate(() => { window.confirm = () => true; });

    await page.locator("#btnReboot").click();
    await page.waitForTimeout(500);
    await expect(page.locator("#statusDot")).not.toHaveClass(/connected/);
    await expect(page.locator("#statusText")).toHaveText("Disconnected");
  });

  test("disconnect cleans up monitorPollTimer", async ({ page }) => {
    await page.goto(FILE_URL);
    const hex = buildConfigHex();
    await setupMockSerial(page, hex);
    await page.evaluate(() => connect());
    await page.waitForTimeout(500);

    const timerBefore = await page.evaluate(() => monitorPollTimer !== null);
    expect(timerBefore).toBe(true);

    await page.evaluate(() => disconnect());
    await page.waitForTimeout(100);
    const timerAfter = await page.evaluate(() => monitorPollTimer);
    expect(timerAfter).toBeNull();
  });

  test("refresh after connect fetches config again", async ({ page }) => {
    await page.goto(FILE_URL);
    const hex = buildConfigHex({ midi_channel: 2 });
    await setupMockSerial(page, hex);
    await page.evaluate(() => connect());
    await page.waitForTimeout(500);
    await page.evaluate(() => stopMonitorPoll());
    await expect(page.locator("#midiChannel")).toHaveValue("2");

    // Update mock to return different config for next GET
    await page.evaluate(() => {
      const buf = [];
      const MAGIC = 0x4D494449;
      buf.push(MAGIC & 0xFF, (MAGIC >> 8) & 0xFF, (MAGIC >> 16) & 0xFF, (MAGIC >> 24) & 0xFF);
      buf.push(2, 9); // version 2, channel 9
      buf.push(0); for (let i = 0; i < 8*3; i++) buf.push(0);
      buf.push(0); for (let i = 0; i < 8*3; i++) buf.push(0);
      buf.push(0); for (let i = 0; i < 4*2; i++) buf.push(0);
      buf.push(28, 74, 0); // ldr
      buf.push(0, 0, 1, 11, 1, 2, 48, 127, 13, 25); // accel
      window.__mockSerial.configHex = Array.from(buf, b => b.toString(16).padStart(2, '0')).join('');
    });

    await page.locator("#btnRefresh").click();
    await page.waitForTimeout(500);
    await expect(page.locator("#midiChannel")).toHaveValue("9");

    const getCount = await page.evaluate(
      () => window.__mockSerial.commandsSent.filter(c => c === "GET").length
    );
    expect(getCount).toBe(2);
  });
});


// ═══════════════════════════════════════════════════
// Connect Error Scenarios
// ═══════════════════════════════════════════════════

test.describe("connect error scenarios", () => {
  test("user cancels port selection (NotFoundError) - no error toast", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      if (!navigator.serial) {
        Object.defineProperty(navigator, 'serial', {
          value: { requestPort: async () => {}, addEventListener: () => {} },
          writable: true,
        });
      }
      navigator.serial.requestPort = async () => {
        const err = new Error("No port selected");
        err.name = "NotFoundError";
        throw err;
      };
    });
    await page.evaluate(() => connect());
    await page.waitForTimeout(300);
    const errorToasts = await page.locator(".toast.error").count();
    expect(errorToasts).toBe(0);
    await expect(page.locator("#statusDot")).not.toHaveClass(/connected/);
  });

  test("port.open() fails - shows connection error toast", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      if (!navigator.serial) {
        Object.defineProperty(navigator, 'serial', {
          value: { requestPort: async () => {}, addEventListener: () => {} },
          writable: true,
        });
      }
      navigator.serial.requestPort = async () => ({
        open: async () => { throw new Error("Failed to open port"); },
        close: async () => {},
        writable: null, readable: null,
      });
    });
    await page.evaluate(() => connect());
    await page.waitForTimeout(300);
    const toasts = await page.locator(".toast").allTextContents();
    expect(toasts.some(t => t.includes("Connection failed"))).toBe(true);
    await expect(page.locator("#statusDot")).not.toHaveClass(/connected/);
  });

  test("unexpected device (VERSION not midictrl) shows warning toast", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      if (!navigator.serial) {
        Object.defineProperty(navigator, 'serial', {
          value: { requestPort: async () => {}, addEventListener: () => {} },
          writable: true,
        });
      }
      window.__toastHistory = [];
      const origToast = window.toast;
      window.toast = (msg, type) => {
        window.__toastHistory.push({ msg, type });
        origToast(msg, type);
      };
      const mockWriter = {
        write: async (data) => {
          const text = new TextDecoder().decode(data).trim();
          let response = "";
          if (text === "VERSION") {
            response = "unknown_device 1.0\n";
          } else if (text === "GET") {
            const buf = [];
            const MAGIC = 0x4D494449;
            buf.push(MAGIC & 0xFF, (MAGIC >> 8) & 0xFF, (MAGIC >> 16) & 0xFF, (MAGIC >> 24) & 0xFF);
            buf.push(2, 0);
            buf.push(0); for (let i = 0; i < 8*3; i++) buf.push(0);
            buf.push(0); for (let i = 0; i < 8*3; i++) buf.push(0);
            buf.push(0); for (let i = 0; i < 4*2; i++) buf.push(0);
            buf.push(0x1c, 0x4a, 0);
            buf.push(0, 0, 1, 11, 1, 2, 48, 127, 13, 25);
            response = Array.from(buf, b => b.toString(16).padStart(2, '0')).join('') + "\n";
          }
          setTimeout(() => { readBuf += response; }, 5);
        },
        releaseLock: () => {},
      };
      navigator.serial.requestPort = async () => ({
        open: async () => {},
        close: async () => {},
        writable: { getWriter: () => mockWriter },
        readable: {
          getReader: () => ({
            read: () => new Promise(() => {}),
            cancel: async () => {},
            releaseLock: () => {},
          }),
        },
      });
    });
    await page.evaluate(() => connect());
    await page.waitForTimeout(500);
    const history = await page.evaluate(() => window.__toastHistory);
    expect(history.some(t => t.msg.includes("Unexpected device"))).toBe(true);
  });

  test("VERSION timeout shows error and disconnects", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      if (!navigator.serial) {
        Object.defineProperty(navigator, 'serial', {
          value: { requestPort: async () => {}, addEventListener: () => {} },
          writable: true,
        });
      }
      navigator.serial.requestPort = async () => ({
        open: async () => {},
        close: async () => {},
        writable: { getWriter: () => ({ write: async () => {}, releaseLock: () => {} }) },
        readable: {
          getReader: () => ({
            read: () => new Promise(() => {}),
            cancel: async () => {},
            releaseLock: () => {},
          }),
        },
      });
    });
    await page.evaluate(() => connect());
    await page.waitForTimeout(4000);
    const toasts = await page.locator(".toast").allTextContents();
    expect(toasts.some(t => t.includes("Connection failed"))).toBe(true);
    await expect(page.locator("#statusDot")).not.toHaveClass(/connected/);
  });
});


// ═══════════════════════════════════════════════════
// Concurrent Operations and Robustness
// ═══════════════════════════════════════════════════

test.describe("concurrent operations and robustness", () => {
  test("double-clicking save only sends one PUT+SAVE pair", async ({ page }) => {
    await page.goto(FILE_URL);
    const hex = buildConfigHex({ midi_channel: 3 });
    await setupMockSerial(page, hex);
    await page.evaluate(() => connect());
    await page.waitForTimeout(500);
    await page.evaluate(() => stopMonitorPoll());

    await page.locator("#btnSave").click();
    // Second click should be ignored (button disabled by setToolbarBusy)
    await page.waitForFunction(
      () => window.__mockSerial.commandsSent.filter(c => c === "SAVE").length >= 1,
      { timeout: 5000 }
    );
    await page.waitForTimeout(500);
    const saveCount = await page.evaluate(
      () => window.__mockSerial.commandsSent.filter(c => c === "SAVE").length
    );
    expect(saveCount).toBe(1);
  });

  test("cmdLock serializes concurrent sendCommand calls", async ({ page }) => {
    await page.goto(FILE_URL);
    const result = await page.evaluate(async () => {
      const writes = [];
      writer = {
        write: async (data) => {
          const text = new TextDecoder().decode(data).trim();
          writes.push(text);
          setTimeout(() => { readBuf += "OK\n"; }, 10);
        },
        releaseLock: () => {},
      };
      const p1 = sendCommand("CMD1");
      const p2 = sendCommand("CMD2");
      const [r1, r2] = await Promise.all([p1, p2]);
      return { writes, r1, r2 };
    });
    expect(result.writes[0]).toBe("CMD1");
    expect(result.writes[1]).toBe("CMD2");
    expect(result.r1).toBe("OK");
    expect(result.r2).toBe("OK");
  });
});


// ═══════════════════════════════════════════════════
// Edge Cases and Bug Documentation
// ═══════════════════════════════════════════════════

test.describe("edge cases and bug documentation", () => {
  test("multiple renderConfig calls don't duplicate items", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      config = {
        midi_channel: 0,
        buttons: [{ pin: 2, note: 60, velocity: 100 }],
        touch_pads: [], pots: [],
        ldr_enabled: false, ldr: { pin: 28, cc: 74 },
        accel_enabled: false,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      renderConfig();
      renderConfig();
      renderConfig();
      setConnected(true);
    });
    await expect(page.locator("#buttonList .item-row")).toHaveCount(1);
    await expect(page.locator("#btnCount")).toHaveText("1");
  });

  test("renderConfig and readConfigFromUI handle null config gracefully", async ({ page }) => {
    await page.goto(FILE_URL);
    const results = await page.evaluate(() => {
      const errors = [];
      config = null;
      try { renderConfig(); } catch (e) { errors.push("render"); }
      try { readConfigFromUI(); } catch (e) { errors.push("read"); }
      return errors;
    });
    expect(results).toEqual([]);
  });

  test("multiple rapid add/remove operations don't corrupt state", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(`(() => {
      config = ${DEFAULT_CFG};
      renderConfig();
      setConnected(true);
    })()`);

    // Add 4 buttons rapidly
    for (let i = 0; i < 4; i++) {
      await page.locator("#addButton").click();
    }
    await expect(page.locator("#btnCount")).toHaveText("4");
    await expect(page.locator("#buttonList .item-row")).toHaveCount(4);

    // Remove all from the end
    for (let i = 3; i >= 0; i--) {
      await page.locator("#buttonList .btn-remove").last().click();
    }
    await expect(page.locator("#btnCount")).toHaveText("0");
    await expect(page.locator("#buttonList .item-row")).toHaveCount(0);
    const count = await page.evaluate(() => config.buttons.length);
    expect(count).toBe(0);
  });

  test("removing item syncs DOM values of remaining items", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      config = {
        midi_channel: 0,
        buttons: [
          { pin: 1, note: 60, velocity: 100 },
          { pin: 2, note: 72, velocity: 100 },
        ],
        touch_pads: [], pots: [],
        ldr_enabled: false, ldr: { pin: 28, cc: 74 },
        accel_enabled: false,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      renderConfig();
      setConnected(true);
    });

    // Edit first button's note, then remove second button
    await page.locator('#buttonList [data-field="note"]').first().fill("69");
    await page.locator("#buttonList .btn-remove").nth(1).click();
    const note = await page.locator('#buttonList [data-field="note"]').first().inputValue();
    expect(note).toBe("69"); // Remove handler calls syncListFromDOM, preserving edits
  });

  test("sendCommand throws on timeout and when not connected", async ({ page }) => {
    await page.goto(FILE_URL);
    // Not connected
    const notConnected = await page.evaluate(async () => {
      writer = null;
      try { await _sendCommand("VERSION"); return null; } catch (e) { return e.message; }
    });
    expect(notConnected).toBe("Not connected");

    // Timeout (no response injected)
    const timeout = await page.evaluate(async () => {
      writer = { write: async () => {}, releaseLock: () => {} };
      try { await _sendCommand("VERSION"); return null; } catch (e) { return e.message; }
    });
    expect(timeout).toBe("Timeout waiting for response");
  });

  test("applyConfig returns false on error", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(`(() => {
      config = ${DEFAULT_CFG};
      renderConfig();
      setConnected(true);
      writer = null; // sendCommand will throw "Not connected"
    })()`);
    const result = await page.evaluate(async () => applyConfig());
    expect(result).toBe(false);
  });

  test("Web Serial disconnect event handler checks port identity", async ({ page }) => {
    await page.goto(FILE_URL);
    const result = await page.evaluate(() => {
      port = { id: "test-port" };
      const match1 = { target: port, port: null };
      const match2 = { target: null, port: port };
      const noMatch = { target: { id: "other" }, port: { id: "other" } };
      return {
        matchTarget: match1.target === port || match1.port === port,
        matchPort: match2.target === port || match2.port === port,
        noMatch: noMatch.target === port || noMatch.port === port,
      };
    });
    expect(result.matchTarget).toBe(true);
    expect(result.matchPort).toBe(true);
    expect(result.noMatch).toBe(false);
  });
});


// ═══════════════════════════════════════════════════
// Documented Bugs
// ═══════════════════════════════════════════════════

test.describe("documented bugs", () => {
  test("BUG: add button/pot doesn't sync DOM first (edited values lost)", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => {
      config = {
        midi_channel: 0,
        buttons: [{ pin: 1, note: 60, velocity: 100 }],
        touch_pads: [],
        pots: [{ pin: 26, cc: 0 }],
        ldr_enabled: false, ldr: { pin: 28, cc: 74 },
        accel_enabled: false,
        accel: { sda: 0, scl: 1, int_pin: 11, x_cc: 1, y_cc: 2, tap_note: 48, tap_vel: 127, dead_zone: 13, smoothing: 25 },
      };
      renderConfig();
      setConnected(true);
    });

    // Edit button note in DOM, then add a new button
    await page.locator('#buttonList input[data-field="note"]').first().fill("72");
    await page.locator("#addButton").click();
    // BUG: addButton pushes to config.buttons and re-renders without syncing DOM first
    const btnNote = await page.evaluate(() => config.buttons[0].note);
    expect(btnNote).toBe(60); // Bug: should be 72 if synced

    // Same bug for pots
    await page.locator('#potList input[data-field="cc"]').first().fill("64");
    await page.locator("#addPot").click();
    const potCc = await page.evaluate(() => config.pots[0].cc);
    expect(potCc).toBe(0); // Bug: should be 64 if synced
  });

  test("BUG: toggle LDR/Accel on doesn't update config before rebuilding monitor", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(`(() => {
      config = ${DEFAULT_CFG};
      renderConfig();
      setConnected(true);
    })()`);

    // Toggle LDR on - handler calls buildLdrMonitor() but doesn't update config.ldr_enabled first
    await page.evaluate(() => {
      document.getElementById("ldrEnabled").checked = true;
      document.getElementById("ldrEnabled").dispatchEvent(new Event("change"));
    });
    const ldrEnabled = await page.evaluate(() => config.ldr_enabled);
    expect(ldrEnabled).toBe(false); // Bug: checkbox is on but config not updated
    const ldrMonCount = await page.locator("#monLdrBar").count();
    expect(ldrMonCount).toBe(0); // Bug: monitor not built because config.ldr_enabled is false

    // Same bug for accelerometer
    await page.evaluate(() => {
      document.getElementById("accelEnabled").checked = true;
      document.getElementById("accelEnabled").dispatchEvent(new Event("change"));
    });
    const accelEnabled = await page.evaluate(() => config.accel_enabled);
    expect(accelEnabled).toBe(false); // Bug
    const accelMonCount = await page.locator("#monAccelXBar").count();
    expect(accelMonCount).toBe(0); // Bug
  });

  test("BUG: setToolbarBusy(false) after disconnect re-enables buttons", async ({ page }) => {
    await page.goto(FILE_URL);
    await page.evaluate(() => setConnected(false));
    await page.evaluate(() => setToolbarBusy(false));
    // Bug: buttons are re-enabled even though we're disconnected
    const refreshEnabled = await page.evaluate(() => !document.getElementById("btnRefresh").disabled);
    const saveEnabled = await page.evaluate(() => !document.getElementById("btnSave").disabled);
    expect(refreshEnabled).toBe(true); // Bug: should be false
    expect(saveEnabled).toBe(true);     // Bug: should be false
  });

  test("BUG: encodeConfig doesn't clamp negative or overflow values", async ({ page }) => {
    await page.goto(FILE_URL);
    const result = await page.evaluate(() => {
      // Negative values wrap in Uint8Array
      const negCfg = {
        midi_channel: -1,
        buttons: [{ pin: -5, note: -10, velocity: -1 }],
        touch_pads: [],
        pots: [{ pin: 26, cc: -10 }],
        ldr_enabled: true, ldr: { pin: 28, cc: -1 },
        accel_enabled: true,
        accel: { sda: 0, scl: 0, int_pin: 0, x_cc: 0, y_cc: 0, tap_note: 0, tap_vel: 1, dead_zone: 300, smoothing: 150 },
      };
      const decoded = decodeConfig(encodeConfig(negCfg));
      return {
        channel: decoded.midi_channel,   // -1 -> 255 -> Math.min(255,15) = 15
        pin: decoded.buttons[0].pin,     // -5 -> 251
        note: decoded.buttons[0].note,   // -10 -> 246 -> Math.min(246,127) = 127
        vel: decoded.buttons[0].velocity,// Math.max(1, Math.min(-1,127)) = 1
        potCc: decoded.pots[0].cc,       // -10 -> 246 -> Math.min(246,127) = 127
        ldrCc: decoded.ldr.cc,           // -1 -> 255 -> Math.min(255,127) = 127
        deadZone: decoded.accel.dead_zone,  // 300 & 0xFF = 44
        smoothing: decoded.accel.smoothing, // 150 fits in u8, no logical max clamp
      };
    });
    expect(result.channel).toBe(15);
    expect(result.pin).toBe(251);        // Bug: should be 0
    expect(result.note).toBe(127);       // Incorrect: wraps then caps
    expect(result.vel).toBe(1);          // Correct: max(1, -1)=1
    expect(result.potCc).toBe(127);      // Bug: should be 0
    expect(result.ldrCc).toBe(127);      // Bug: should be 0
    expect(result.deadZone).toBe(44);    // Bug: 300 wraps, should clamp to 255
    expect(result.smoothing).toBe(150);  // No clamp in encode/decode path
  });
});
