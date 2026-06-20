// Dev-only helper: generate a 1024x1024 PNG source icon for `tauri icon`.
// Renders the Keyhaven mark — a gradient-blue rounded tile with a white keyhole —
// on a FULLY TRANSPARENT background (the rounded tile is the icon; there is no
// surrounding white/off-white box). Not shipped; only the generated icon outputs
// are bundled. Mirrors the vector logo in `public/keyhaven.svg`.
import { deflateSync, crc32 } from "node:zlib";
import { writeFileSync } from "node:fs";

const SIZE = 1024;

// Palette
const TRANSPARENT = [0, 0, 0, 0];
const BLUE_TOP = [74, 144, 208]; // #4a90d0 — gradient top
const BLUE_BOT = [47, 109, 168]; // #2f6da8 — gradient bottom
const WHITE = [255, 255, 255, 255];

function lerp(a, b, t) {
  return Math.round(a + (b - a) * t);
}

function inRoundedRect(x, y, x0, y0, x1, y1, r) {
  if (x < x0 || x > x1 || y < y0 || y > y1) return false;
  const cx = Math.min(Math.max(x, x0 + r), x1 - r);
  const cy = Math.min(Math.max(y, y0 + r), y1 - r);
  const dx = x - cx;
  const dy = y - cy;
  return dx * dx + dy * dy <= r * r || x >= x0 + r || x <= x1 - r || y >= y0 + r || y <= y1 - r
    ? insideRounded(x, y, x0, y0, x1, y1, r)
    : false;
}

function insideRounded(x, y, x0, y0, x1, y1, r) {
  if (x >= x0 + r && x <= x1 - r) return y >= y0 && y <= y1;
  if (y >= y0 + r && y <= y1 - r) return x >= x0 && x <= x1;
  const corners = [
    [x0 + r, y0 + r],
    [x1 - r, y0 + r],
    [x0 + r, y1 - r],
    [x1 - r, y1 - r],
  ];
  for (const [cx, cy] of corners) {
    const dx = x - cx;
    const dy = y - cy;
    if (dx * dx + dy * dy <= r * r) return true;
  }
  return false;
}

function pixel(x, y) {
  // Everything outside the rounded full-bleed tile is transparent — this is
  // what removes the old white/off-white box around the mark.
  const r = SIZE * 0.234; // matches rx=15 on the 64px vector tile
  if (!insideRounded(x, y, 0, 0, SIZE, SIZE, r)) return TRANSPARENT;

  // Vertical blue gradient fills the tile.
  const t = y / SIZE;
  let c = [
    lerp(BLUE_TOP[0], BLUE_BOT[0], t),
    lerp(BLUE_TOP[1], BLUE_BOT[1], t),
    lerp(BLUE_TOP[2], BLUE_BOT[2], t),
    255,
  ];

  // Keyhole head (ring/disc), drawn in white over the tile.
  const headX = SIZE * 0.5;
  const headY = SIZE * 0.39;
  const headR = SIZE * 0.133;
  const dx = x - headX;
  const dy = y - headY;
  if (dx * dx + dy * dy <= headR * headR) return WHITE;

  // Tapered keyhole stem widening toward the bottom.
  const topY = SIZE * 0.453;
  const botY = SIZE * 0.719;
  if (y >= topY && y <= botY) {
    const tt = (y - topY) / (botY - topY);
    const halfW = SIZE * 0.0547 + tt * (SIZE * 0.1016 - SIZE * 0.0547);
    if (x >= headX - halfW && x <= headX + halfW) return WHITE;
  }

  return c;
}

// Build raw RGBA scanlines with filter byte 0 per row.
const rowBytes = 1 + SIZE * 4;
const raw = Buffer.alloc(rowBytes * SIZE);
for (let y = 0; y < SIZE; y++) {
  const rowStart = y * rowBytes;
  raw[rowStart] = 0; // filter: none
  for (let x = 0; x < SIZE; x++) {
    const [r, g, b, a] = pixel(x, y);
    const o = rowStart + 1 + x * 4;
    raw[o] = r;
    raw[o + 1] = g;
    raw[o + 2] = b;
    raw[o + 3] = a;
  }
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const typeBuf = Buffer.from(type, "ascii");
  const body = Buffer.concat([typeBuf, data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body) >>> 0, 0);
  return Buffer.concat([len, body, crc]);
}

const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 6; // color type RGBA
ihdr[10] = 0; // compression
ihdr[11] = 0; // filter
ihdr[12] = 0; // interlace

const idat = deflateSync(raw, { level: 9 });
const png = Buffer.concat([
  sig,
  chunk("IHDR", ihdr),
  chunk("IDAT", idat),
  chunk("IEND", Buffer.alloc(0)),
]);

const out = process.argv[2] || "app-icon.png";
writeFileSync(out, png);
console.log(`Wrote ${out} (${png.length} bytes)`);
