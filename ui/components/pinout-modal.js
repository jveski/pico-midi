// Pinout Modal — shows an SVG of the Raspberry Pi Pico with a specific pin highlighted.
//
// Usage:
//   const modal = document.querySelector("pinout-modal");
//   modal.show(26);  // highlights GP26

// ── Pin Data ──
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

// Build a map from gpio number to physical pin number
const GPIO_TO_PHYSICAL = {};
for (const [phys, , gpio] of LEFT_PINS.concat(RIGHT_PINS)) {
  if (gpio !== null) GPIO_TO_PHYSICAL[gpio] = phys;
}

// ── Pin classification for color coding ──
function pinClass(label) {
  if (label === "GND" || label === "AGND") return "gnd";
  if (label === "VBUS" || label === "VSYS" || label === "3V3" || label === "3V3_EN" || label === "ADC_VREF") return "pwr";
  if (label === "RUN") return "special";
  return "gpio";
}

// ── SVG Generation ──

const BOARD_W = 200;
const BOARD_H = 490;
const PIN_RADIUS = 7;
const PIN_SPACING = 22;
const PIN_Y_START = 36;
const PIN_X_LEFT = 18;
const PIN_X_RIGHT = BOARD_W - 18;
const USB_W = 50;
const USB_H = 16;

function buildSVG(highlightGpio) {
  const highlightPhys = highlightGpio != null ? GPIO_TO_PHYSICAL[highlightGpio] : null;

  // Full SVG width includes labels on both sides
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

  // Helper to draw a pin + label
  function drawPin(physPin, label, cx, cy, side) {
    const isHighlight = physPin === highlightPhys;
    const cls = pinClass(label);

    // Pin colors
    let fill, stroke, textFill;
    if (isHighlight) {
      fill = "#0f0";
      stroke = "#0f0";
      textFill = "#0f0";
    } else if (cls === "gnd") {
      fill = "#333";
      stroke = "#555";
      textFill = "#666";
    } else if (cls === "pwr") {
      fill = "#511";
      stroke = "#a33";
      textFill = "#a33";
    } else if (cls === "special") {
      fill = "#333";
      stroke = "#777";
      textFill = "#777";
    } else {
      fill = "#222";
      stroke = "#888";
      textFill = "#ccc";
    }

    // Glow effect for highlighted pin
    if (isHighlight) {
      svg += `<circle cx="${cx}" cy="${cy}" r="${PIN_RADIUS + 4}" fill="none" stroke="#0f0" stroke-width="1" opacity="0.4"/>`;
      svg += `<circle cx="${cx}" cy="${cy}" r="${PIN_RADIUS + 8}" fill="none" stroke="#0f0" stroke-width="0.5" opacity="0.2"/>`;
    }

    // Pin circle
    svg += `<circle cx="${cx}" cy="${cy}" r="${PIN_RADIUS}" fill="${fill}" stroke="${stroke}" stroke-width="1.5"/>`;

    // Physical pin number inside the circle
    svg += `<text x="${cx}" y="${cy}" text-anchor="middle" dominant-baseline="central" fill="${isHighlight ? '#000' : '#888'}" font-size="7" font-family="monospace" font-weight="${isHighlight ? '700' : '400'}">${physPin}</text>`;

    // Label outside the board
    const labelX = side === "left" ? boardX - 10 : boardX + BOARD_W + 10;
    const anchor = side === "left" ? "end" : "start";
    svg += `<text x="${labelX}" y="${cy}" text-anchor="${anchor}" dominant-baseline="central" fill="${textFill}" font-size="10" font-family="monospace" font-weight="${isHighlight ? '700' : '400'}">${label}</text>`;
  }

  // Draw left-side pins (top to bottom)
  LEFT_PINS.forEach(([phys, label, ], i) => {
    const cx = boardX + PIN_X_LEFT;
    const cy = boardY + PIN_Y_START + i * PIN_SPACING;
    drawPin(phys, label, cx, cy, "left");
  });

  // Draw right-side pins (top to bottom, physical 40 down to 21)
  RIGHT_PINS.forEach(([phys, label, ], i) => {
    const cx = boardX + PIN_X_RIGHT;
    const cy = boardY + PIN_Y_START + i * PIN_SPACING;
    drawPin(phys, label, cx, cy, "right");
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

// ── Web Component ──

export class PinoutModal extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;

    this.innerHTML =
      '<div class="pinout-backdrop">' +
        '<div class="pinout-dialog">' +
          '<div class="pinout-header">' +
            '<span class="pinout-title">Pinout</span>' +
            '<button class="pinout-close">&times;</button>' +
          '</div>' +
          '<div class="pinout-body"></div>' +
        '</div>' +
      '</div>';

    this._backdrop = this.querySelector(".pinout-backdrop");
    this._body = this.querySelector(".pinout-body");
    this._title = this.querySelector(".pinout-title");

    // Close on backdrop click
    this._backdrop.addEventListener("click", (e) => {
      if (e.target === this._backdrop) this.hide();
    });

    // Close on X button
    this.querySelector(".pinout-close").addEventListener("click", () => this.hide());

    // Close on Escape
    this._onKeyDown = (e) => {
      if (e.key === "Escape") this.hide();
    };
  }

  show(gpioNumber) {
    if (!this._body) return;
    this._body.innerHTML = buildSVG(gpioNumber);
    this._title.textContent = gpioNumber != null ? `Pinout — GP${gpioNumber}` : "Pinout";
    this._backdrop.classList.add("visible");
    document.addEventListener("keydown", this._onKeyDown);
  }

  hide() {
    if (!this._backdrop) return;
    this._backdrop.classList.remove("visible");
    document.removeEventListener("keydown", this._onKeyDown);
  }
}

customElements.define("pinout-modal", PinoutModal);
