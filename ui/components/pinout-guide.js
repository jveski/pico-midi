import { BaseElement } from "./helpers.js";

// Physical pin 1 is top-left (USB up), counting down left side, then up right side.
// Each entry: [physicalPin, label, gpio]  (gpio is null for non-GPIO pins)

const LEFT_PINS = [
  [1,  "GP0",       0],
  [2,  "GP1",       1],
  [3,  "GND",       null],
  [4,  "GP2",       2],
  [5,  "GP3",       3],
  [6,  "GP4",       4],
  [7,  "GP5",       5],
  [8,  "GND",       null],
  [9,  "GP6",       6],
  [10, "GP7",       7],
  [11, "GP8",       8],
  [12, "GP9",       9],
  [13, "GND",       null],
  [14, "GP10",      10],
  [15, "GP11",      11],
  [16, "GP12",      12],
  [17, "GP13",      13],
  [18, "GND",       null],
  [19, "GP14",      14],
  [20, "GP15",      15],
];

const RIGHT_PINS = [
  [40, "VBUS",      null],
  [39, "VSYS",      null],
  [38, "GND",       null],
  [37, "3V3_EN",    null],
  [36, "3V3",       null],
  [35, "ADC_VREF",  null],
  [34, "GP28/A2",   28],
  [33, "AGND",      null],
  [32, "GP27/A1",   27],
  [31, "GP26/A0",   26],
  [30, "RUN",       null],
  [29, "GP22",      22],
  [28, "GND",       null],
  [27, "GP21",      21],
  [26, "GP20",      20],
  [25, "GP19",      19],
  [24, "GP18",      18],
  [23, "GND",       null],
  [22, "GP17",      17],
  [21, "GP16",      16],
];

function pinClass(label) {
  if (label === "GND" || label === "AGND") return "gnd";
  if (label === "VBUS" || label === "VSYS" || label === "3V3" || label === "3V3_EN" || label === "ADC_VREF") return "pwr";
  if (label === "RUN") return "special";
  return "gpio";
}

// ── Input assignment colors ──
const ASSIGN_COLORS = {
  button:  { fill: "#062", stroke: "#0f0", text: "#0f0", tag: "#0f0" },
  touch:   { fill: "#330", stroke: "#ff0", text: "#ff0", tag: "#ff0" },
  pot:     { fill: "#203", stroke: "#c6f", text: "#c6f", tag: "#c6f" },
  ldr:     { fill: "#230", stroke: "#f80", text: "#f80", tag: "#f80" },
  accel:   { fill: "#024", stroke: "#0af", text: "#0af", tag: "#0af" },
};

/**
 * Build a gpio→assignment map from the current config.
 */
function buildAssignments(cfg) {
  const map = {};
  if (!cfg) return map;

  cfg.buttons.forEach((b, i) => {
    map[b.pin] = { type: "button", label: `Btn ${i + 1}`, colors: ASSIGN_COLORS.button };
  });
  cfg.touch_pads.forEach((t, i) => {
    map[t.pin] = { type: "touch", label: `Touch ${i + 1}`, colors: ASSIGN_COLORS.touch };
  });
  cfg.pots.forEach((p, i) => {
    map[p.pin] = { type: "pot", label: `Pot ${i + 1}`, colors: ASSIGN_COLORS.pot };
  });
  if (cfg.ldr_enabled) {
    map[cfg.ldr.pin] = { type: "ldr", label: "LDR", colors: ASSIGN_COLORS.ldr };
  }
  if (cfg.accel.enabled) {
    map[2] = { type: "accel", label: "Accel SDA", colors: ASSIGN_COLORS.accel };
    map[3] = { type: "accel", label: "Accel SCL", colors: ASSIGN_COLORS.accel };
  }

  return map;
}

const BOARD_W = 200;
const BOARD_H = 490;
const PIN_RADIUS = 7;
const PIN_SPACING = 22;
const PIN_Y_START = 36;
const PIN_X_LEFT = 18;
const PIN_X_RIGHT = BOARD_W - 18;
const USB_W = 50;
const USB_H = 16;

function buildSVG(assignments) {
  const svgW = 480;
  const svgH = BOARD_H + 30;
  const boardX = (svgW - BOARD_W) / 2;
  const boardY = 15;

  let svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${svgW} ${svgH}" width="${svgW}" height="${svgH}" style="max-width:100%;height:auto">`;

  // Board body
  svg += `<rect x="${boardX}" y="${boardY}" width="${BOARD_W}" height="${BOARD_H}" rx="8" fill="#1a3a1a" stroke="#2d5a2d" stroke-width="2"/>`;

  // USB port
  const usbX = boardX + (BOARD_W - USB_W) / 2;
  svg += `<rect x="${usbX}" y="${boardY - 4}" width="${USB_W}" height="${USB_H}" rx="3" fill="#444" stroke="#666" stroke-width="1.5"/>`;
  svg += `<text x="${usbX + USB_W / 2}" y="${boardY + USB_H / 2 - 1}" text-anchor="middle" dominant-baseline="central" fill="#999" font-size="7" font-family="monospace">USB</text>`;

  // Raspberry Pi Pico label
  svg += `<text x="${boardX + BOARD_W / 2}" y="${boardY + BOARD_H - 10}" text-anchor="middle" fill="#2d5a2d" font-size="10" font-family="monospace" font-weight="700">RASPBERRY PI PICO</text>`;

  function drawPin(physPin, label, gpio, cx, cy, side) {
    const assign = gpio != null ? assignments[gpio] : null;
    const cls = pinClass(label);

    let fill, stroke, textFill, displayLabel, tagLabel;

    if (assign) {
      fill = assign.colors.fill;
      stroke = assign.colors.stroke;
      textFill = assign.colors.text;
      displayLabel = label;
      tagLabel = assign.label;
    } else if (cls === "gnd") {
      fill = "#333"; stroke = "#555"; textFill = "#666";
      displayLabel = label;
    } else if (cls === "pwr") {
      fill = "#511"; stroke = "#a33"; textFill = "#a33";
      displayLabel = label;
    } else if (cls === "special") {
      fill = "#333"; stroke = "#777"; textFill = "#777";
      displayLabel = label;
    } else {
      fill = "#222"; stroke = "#888"; textFill = "#555";
      displayLabel = label;
    }

    // Glow for assigned pins
    if (assign) {
      svg += `<circle cx="${cx}" cy="${cy}" r="${PIN_RADIUS + 4}" fill="none" stroke="${assign.colors.stroke}" stroke-width="1" opacity="0.3"/>`;
    }

    // Pin circle
    svg += `<circle cx="${cx}" cy="${cy}" r="${PIN_RADIUS}" fill="${fill}" stroke="${stroke}" stroke-width="1.5"/>`;

    // Physical pin number inside the circle
    svg += `<text x="${cx}" y="${cy}" text-anchor="middle" dominant-baseline="central" fill="${assign ? '#000' : '#888'}" font-size="7" font-family="monospace" font-weight="${assign ? '700' : '400'}">${physPin}</text>`;

    // Label outside the board — GPIO name
    const labelX = side === "left" ? boardX - 10 : boardX + BOARD_W + 10;
    const anchor = side === "left" ? "end" : "start";
    svg += `<text x="${labelX}" y="${cy}" text-anchor="${anchor}" dominant-baseline="central" fill="${textFill}" font-size="10" font-family="monospace" font-weight="${assign ? '700' : '400'}">${displayLabel}</text>`;

    // Assignment tag — input name on the far side
    if (tagLabel) {
      const tagX = side === "left" ? 8 : svgW - 8;
      const tagAnchor = side === "left" ? "start" : "end";
      svg += `<text x="${tagX}" y="${cy}" text-anchor="${tagAnchor}" dominant-baseline="central" fill="${assign.colors.tag}" font-size="9" font-family="monospace" font-weight="700">${tagLabel}</text>`;
    }
  }

  // Draw left-side pins
  LEFT_PINS.forEach(([phys, label, gpio], i) => {
    const cx = boardX + PIN_X_LEFT;
    const cy = boardY + PIN_Y_START + i * PIN_SPACING;
    drawPin(phys, label, gpio, cx, cy, "left");
  });

  // Draw right-side pins
  RIGHT_PINS.forEach(([phys, label, gpio], i) => {
    const cx = boardX + PIN_X_RIGHT;
    const cy = boardY + PIN_Y_START + i * PIN_SPACING;
    drawPin(phys, label, gpio, cx, cy, "right");
  });

  // Debug header at bottom
  const dbgY = boardY + BOARD_H - 30;
  const dbgPins = ["SWCLK", "GND", "SWDIO"];
  dbgPins.forEach((label, i) => {
    const cx = boardX + BOARD_W / 2 + (i - 1) * 24;
    svg += `<circle cx="${cx}" cy="${dbgY}" r="4" fill="#222" stroke="#555" stroke-width="1"/>`;
    svg += `<text x="${cx}" y="${dbgY + 12}" text-anchor="middle" fill="#555" font-size="6" font-family="monospace">${label}</text>`;
  });

  svg += `</svg>`;
  return svg;
}

function buildLegend(assignments) {
  const types = new Set(Object.values(assignments).map(a => a.type));
  if (types.size === 0) return "";

  const entries = [];
  const order = ["button", "touch", "pot", "ldr", "accel"];
  const labels = { button: "Button", touch: "Touch Pad", pot: "Pot", ldr: "LDR", accel: "Accelerometer" };
  for (const t of order) {
    if (types.has(t)) {
      const c = ASSIGN_COLORS[t];
      entries.push(`<span class="pinout-legend-item"><span class="pinout-legend-dot" style="background:${c.stroke}"></span>${labels[t]}</span>`);
    }
  }
  return `<div class="pinout-legend">${entries.join("")}</div>`;
}

export class PinoutGuide extends BaseElement {
  init() {
    this._container = this.querySelector(".pinout-guide-body");
  }

  update(config) {
    if (!this._container) return;
    const assignments = buildAssignments(config);
    this._container.innerHTML = buildLegend(assignments) + buildSVG(assignments);
  }
}

customElements.define("pinout-guide", PinoutGuide);
