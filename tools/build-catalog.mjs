#!/usr/bin/env node
// =============================================================================
//  Keyhaven — Service Catalog Builder  (DEV-ONLY TOOLING)
// =============================================================================
//
//  ⚠️  THIS SCRIPT IS DEVELOPMENT-ONLY AND IS **NEVER SHIPPED** TO USERS.
//
//  It is not bundled into the Tauri application, is not referenced at runtime,
//  and performs no cryptography. Only the *output* it produces is bundled:
//
//      • src-tauri/catalog/services-catalog.json   (the curated catalog)
//      • src-tauri/catalog/icons/*.svg             (one placeholder icon each)
//
//  The Rust catalog loader (task 7.2) reads the generated JSON/icons at
//  compile/runtime; this generator is purely a build-time convenience for
//  curating the data set under version control. (Requirements 12.1, 12.5)
//
//  OFFLINE BY DESIGN (runtime): the *app* never makes network calls. This
//  builder, however, fetches real brand logos from Simple Icons (CC0) at BUILD
//  time and bundles the resulting SVGs, so the shipped app shows recognizable
//  service logos while still running fully offline. Services without a Simple
//  Icons match fall back to a deterministic letter-monogram SVG generated
//  locally.
//
//  Run with:
//      node tools/build-catalog.mjs
//
//  Output JSON shape — a top-level JSON array of service objects (chosen for
//  simplicity; the Rust loader deserializes Vec<CatalogService>):
//
//      [
//        {
//          "id": "facebook",
//          "name": "Facebook",
//          "icon": "facebook.svg",
//          "aliases": ["fb", "meta"],
//          "recommended_fields": [
//            { "label": "Email",    "type": "email",    "secret": false },
//            { "label": "Password", "type": "password", "secret": true  }
//          ]
//        },
//        ...
//      ]
//
//  Field `type` values MUST be one of the snake_case values matching the Rust
//  `FieldType` enum:
//      email | username | password | phone | url | text | note
//      | totp_secret | recovery_code
// =============================================================================

import { mkdir, writeFile, rm, readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join, extname } from "node:path";

// ----------------------------------------------------------------------------
// Paths (resolved relative to this script, so it works from any CWD).
// ----------------------------------------------------------------------------
const __dirname = dirname(fileURLToPath(import.meta.url));
const WORKSPACE_ROOT = join(__dirname, "..");
const OUTPUT_DIR = join(WORKSPACE_ROOT, "src-tauri", "catalog");
const ICONS_DIR = join(OUTPUT_DIR, "icons");
const CATALOG_JSON = join(OUTPUT_DIR, "services-catalog.json");

// Hand-curated, high-quality logo files bundled in the repo. When a service id
// appears here, its icon is taken from this local image (embedded as a data
// URL) instead of being fetched online — used for brands whose live favicon is
// low-resolution/blurry (e.g. Coins.ph, PLDT). These pre-processed PNGs were
// cropped to a square and downscaled to 256x256 for crisp circular rendering.
const ICON_OVERRIDES_DIR = join(__dirname, "icon-overrides");
const LOCAL_ICON_OVERRIDES = Object.freeze({
  coinsph: "coinsph.png",
  pldt: "pldt.png",
});

/** Read a bundled override image and return it as a data URL (build-time only). */
async function localIconDataUrl(fileName) {
  const buf = await readFile(join(ICON_OVERRIDES_DIR, fileName));
  const ext = extname(fileName).toLowerCase();
  const ct = ext === ".png" ? "image/png" : ext === ".svg" ? "image/svg+xml" : "image/jpeg";
  return `data:${ct};base64,${buf.toString("base64")}`;
}

// ----------------------------------------------------------------------------
// Field-type constants (must match the Rust `FieldType` snake_case wire values).
// ----------------------------------------------------------------------------
const T = Object.freeze({
  EMAIL: "email",
  USERNAME: "username",
  PASSWORD: "password",
  PHONE: "phone",
  URL: "url",
  TEXT: "text",
  NOTE: "note",
  TOTP: "totp_secret",
  RECOVERY: "recovery_code",
});

const VALID_TYPES = new Set(Object.values(T));

// ----------------------------------------------------------------------------
// Field templates — small helpers to keep the curated data readable.
// ----------------------------------------------------------------------------
const f = (label, type, secret = false) => ({ label, type, secret });

const EMAIL = () => f("Email", T.EMAIL, false);
const USERNAME = () => f("Username", T.USERNAME, false);
const PHONE = () => f("Phone", T.PHONE, false);
const URLF = () => f("Website", T.URL, false);
const PASSWORD = () => f("Password", T.PASSWORD, true);
const TOTP = () => f("2FA secret", T.TOTP, true);
const RECOVERY = () => f("Recovery codes", T.RECOVERY, true);

// Common recommended-field presets.
// - Email-login service with 2FA + recovery codes (typical big consumer/dev svc)
const emailLogin2fa = () => [EMAIL(), PASSWORD(), TOTP(), RECOVERY()];
// - Email-login, no recovery codes surfaced
const emailLogin = () => [EMAIL(), PASSWORD(), TOTP()];
// - Username-or-email login (social / forums)
const userOrEmail2fa = () => [USERNAME(), EMAIL(), PASSWORD(), TOTP()];
// - Username-first login
const userLogin2fa = () => [USERNAME(), PASSWORD(), TOTP()];
// - Phone-first login (messengers)
const phoneLogin = () => [PHONE(), PASSWORD()];
// - Financial / crypto: extra security weight
const financial = () => [EMAIL(), PASSWORD(), TOTP(), RECOVERY()];

// ----------------------------------------------------------------------------
// Curated catalog of ~50 popular GLOBAL services.
// ----------------------------------------------------------------------------
const SERVICES = [
  { id: "google", name: "Google", aliases: ["gmail", "gsuite", "google workspace", "youtube account"], recommended_fields: emailLogin2fa() },
  { id: "facebook", name: "Facebook", aliases: ["fb", "meta"], recommended_fields: userOrEmail2fa() },
  { id: "instagram", name: "Instagram", aliases: ["ig", "insta", "meta"], recommended_fields: userOrEmail2fa() },
  { id: "threads", name: "Threads", aliases: ["meta", "instagram threads", "ig threads"], recommended_fields: userOrEmail2fa() },
  { id: "x", name: "X (Twitter)", aliases: ["twitter", "tweet"], recommended_fields: userOrEmail2fa() },
  { id: "linkedin", name: "LinkedIn", aliases: ["li"], recommended_fields: emailLogin2fa() },
  { id: "microsoft", name: "Microsoft", aliases: ["msft", "outlook", "hotmail", "live", "office365", "azure ad"], recommended_fields: emailLogin2fa() },
  { id: "apple", name: "Apple", aliases: ["icloud", "apple id", "itunes"], recommended_fields: emailLogin2fa() },
  { id: "amazon", name: "Amazon", aliases: ["aws account", "prime"], recommended_fields: emailLogin2fa() },
  { id: "netflix", name: "Netflix", aliases: ["nflx"], recommended_fields: emailLogin() },
  { id: "spotify", name: "Spotify", aliases: ["music"], recommended_fields: emailLogin() },
  { id: "github", name: "GitHub", aliases: ["gh", "git"], recommended_fields: userLogin2fa().concat(RECOVERY()) },
  { id: "gitlab", name: "GitLab", aliases: ["git"], recommended_fields: userOrEmail2fa().concat(RECOVERY()) },
  { id: "dropbox", name: "Dropbox", aliases: ["db", "storage"], recommended_fields: emailLogin2fa() },
  { id: "slack", name: "Slack", aliases: ["chat", "work"], recommended_fields: [EMAIL(), URLF(), PASSWORD(), TOTP()] },
  { id: "discord", name: "Discord", aliases: ["dis", "chat", "gaming"], recommended_fields: [USERNAME(), EMAIL(), PASSWORD(), TOTP(), RECOVERY()] },
  { id: "reddit", name: "Reddit", aliases: ["sub", "forum"], recommended_fields: userOrEmail2fa() },
  { id: "paypal", name: "PayPal", aliases: ["pp", "payments"], recommended_fields: financial() },
  { id: "ebay", name: "eBay", aliases: ["auction", "shop"], recommended_fields: emailLogin() },
  { id: "whatsapp", name: "WhatsApp", aliases: ["wa", "meta", "messenger"], recommended_fields: phoneLogin() },
  { id: "telegram", name: "Telegram", aliases: ["tg", "messenger"], recommended_fields: [PHONE(), PASSWORD()] },
  { id: "signal", name: "Signal", aliases: ["messenger", "private"], recommended_fields: [PHONE(), f("PIN", T.PASSWORD, true)] },
  { id: "zoom", name: "Zoom", aliases: ["meeting", "video"], recommended_fields: emailLogin() },
  { id: "steam", name: "Steam", aliases: ["valve", "games"], recommended_fields: [USERNAME(), PASSWORD(), TOTP(), RECOVERY()] },
  { id: "twitch", name: "Twitch", aliases: ["stream", "amazon"], recommended_fields: userOrEmail2fa() },
  { id: "tiktok", name: "TikTok", aliases: ["bytedance", "douyin"], recommended_fields: userOrEmail2fa() },
  { id: "pinterest", name: "Pinterest", aliases: ["pin", "boards"], recommended_fields: emailLogin() },
  { id: "snapchat", name: "Snapchat", aliases: ["snap"], recommended_fields: [USERNAME(), PHONE(), PASSWORD()] },
  { id: "yahoo", name: "Yahoo", aliases: ["ymail", "yahoo mail"], recommended_fields: emailLogin2fa() },
  { id: "protonmail", name: "Proton", aliases: ["protonmail", "proton mail", "proton vpn"], recommended_fields: [EMAIL(), PASSWORD(), f("Mailbox password", T.PASSWORD, true), TOTP()] },
  { id: "adobe", name: "Adobe", aliases: ["creative cloud", "photoshop", "acrobat"], recommended_fields: emailLogin2fa() },
  { id: "atlassian", name: "Atlassian", aliases: ["jira", "confluence", "trello", "bitbucket"], recommended_fields: emailLogin2fa() },
  { id: "notion", name: "Notion", aliases: ["notes", "docs"], recommended_fields: emailLogin() },
  { id: "figma", name: "Figma", aliases: ["design"], recommended_fields: emailLogin() },
  { id: "stripe", name: "Stripe", aliases: ["payments", "dashboard"], recommended_fields: financial() },
  { id: "shopify", name: "Shopify", aliases: ["store", "ecommerce"], recommended_fields: [EMAIL(), URLF(), PASSWORD(), TOTP()] },
  { id: "wordpress", name: "WordPress", aliases: ["wp", "blog", "automattic"], recommended_fields: [USERNAME(), URLF(), PASSWORD()] },
  { id: "cloudflare", name: "Cloudflare", aliases: ["cf", "dns", "cdn"], recommended_fields: emailLogin2fa() },
  { id: "aws", name: "Amazon Web Services", aliases: ["aws", "amazon web services", "iam"], recommended_fields: [f("Account ID", T.TEXT, false), USERNAME(), PASSWORD(), TOTP()] },
  { id: "digitalocean", name: "DigitalOcean", aliases: ["do", "droplet", "cloud"], recommended_fields: emailLogin2fa() },
  { id: "heroku", name: "Heroku", aliases: ["dyno", "salesforce"], recommended_fields: emailLogin2fa() },
  { id: "tumblr", name: "Tumblr", aliases: ["blog"], recommended_fields: emailLogin() },
  { id: "vimeo", name: "Vimeo", aliases: ["video"], recommended_fields: emailLogin() },
  { id: "soundcloud", name: "SoundCloud", aliases: ["sc", "music", "audio"], recommended_fields: emailLogin() },
  { id: "airbnb", name: "Airbnb", aliases: ["abnb", "travel", "stay"], recommended_fields: [EMAIL(), PHONE(), PASSWORD()] },
  { id: "uber", name: "Uber", aliases: ["ride", "eats"], recommended_fields: [EMAIL(), PHONE(), PASSWORD()] },
  { id: "booking", name: "Booking.com", aliases: ["booking", "travel", "hotel"], recommended_fields: emailLogin() },
  { id: "coinbase", name: "Coinbase", aliases: ["crypto", "exchange", "btc"], recommended_fields: financial() },
  { id: "binance", name: "Binance", aliases: ["crypto", "exchange", "bnb"], recommended_fields: financial() },
  { id: "kraken", name: "Kraken", aliases: ["crypto", "exchange"], recommended_fields: financial() },
  { id: "ebay-classifieds", name: "Etsy", aliases: ["etsy", "handmade", "shop"], recommended_fields: emailLogin() },
  { id: "yandex", name: "Yandex", aliases: ["yandex mail", "ru"], recommended_fields: emailLogin2fa() },
  { id: "wechat", name: "WeChat", aliases: ["weixin", "tencent", "messenger"], recommended_fields: phoneLogin() },

  // ---- Batch update: gaming, PH services, crypto, and more ----
  { id: "socialclub", name: "Social Club", aliases: ["rockstar", "rockstar games", "rgl", "gta", "rdr", "gta online"], recommended_fields: emailLogin2fa() },
  { id: "ea", name: "EA / Origin", aliases: ["origin", "ea app", "electronic arts", "ea play"], recommended_fields: emailLogin2fa() },
  { id: "sellix", name: "Sellix", aliases: ["sell", "selling", "ecommerce", "store"], recommended_fields: [EMAIL(), PASSWORD(), TOTP()] },
  { id: "coinsph", name: "Coins.ph", aliases: ["coins", "coins ph", "crypto", "wallet", "ph", "philippines"], recommended_fields: financial() },
  { id: "pldt", name: "PLDT", aliases: ["pldt home", "wifi", "internet", "fibr", "ph", "philippines"], recommended_fields: [EMAIL(), PHONE(), PASSWORD()] },
  { id: "rcbc", name: "RCBC", aliases: ["rizal commercial banking", "rcbc online", "bank", "ph", "philippines"], recommended_fields: [USERNAME(), EMAIL(), PASSWORD(), TOTP()] },
  { id: "okx", name: "OKX", aliases: ["okex", "crypto", "exchange"], recommended_fields: financial() },
  { id: "metamask", name: "MetaMask", aliases: ["wallet", "crypto", "web3", "eth", "ethereum"], recommended_fields: [f("Wallet password", T.PASSWORD, true), f("Secret Recovery Phrase", T.RECOVERY, true)] },
  { id: "chatgpt", name: "ChatGPT", aliases: ["openai", "gpt", "chat gpt"], recommended_fields: emailLogin2fa() },
  { id: "payoneer", name: "Payoneer", aliases: ["payments", "payout", "wallet"], recommended_fields: financial() },
  { id: "kiro", name: "Kiro", aliases: ["kiro dev", "ai ide", "ide"], recommended_fields: emailLogin2fa() },
  { id: "datablitz", name: "DataBlitz", aliases: ["data blitz", "games", "gaming store", "ph", "philippines"], recommended_fields: emailLogin() },
  { id: "onlinejobs", name: "OnlineJobs.ph", aliases: ["online jobs", "ojph", "remote work", "freelance", "ph", "philippines"], recommended_fields: emailLogin() },
];

// ----------------------------------------------------------------------------
// Deterministic placeholder SVG icon generator.
//
// Produces a small, self-contained rounded-square badge with the service's
// initial letter. Color is derived deterministically from the service id so
// runs are reproducible and diffs are stable.
// ----------------------------------------------------------------------------
function colorForId(id) {
  // Simple deterministic hash → hue. No randomness so output is stable.
  let hash = 0;
  for (let i = 0; i < id.length; i++) {
    hash = (hash * 31 + id.charCodeAt(i)) >>> 0;
  }
  const hue = hash % 360;
  return {
    bg: `hsl(${hue}, 55%, 45%)`,
    bgDark: `hsl(${hue}, 55%, 38%)`,
  };
}

function escapeXml(s) {
  return s.replace(/[<>&'"]/g, (c) =>
    ({ "<": "&lt;", ">": "&gt;", "&": "&amp;", "'": "&apos;", '"': "&quot;" }[c]),
  );
}

function svgForService(service) {
  const initial = escapeXml((service.name.trim()[0] || "?").toUpperCase());
  const { bg, bgDark } = colorForId(service.id);
  // 64x64 viewBox, rounded square, centered initial. Deterministic & offline.
  return `<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64" role="img" aria-label="${escapeXml(service.name)}">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="${bg}"/>
      <stop offset="1" stop-color="${bgDark}"/>
    </linearGradient>
  </defs>
  <rect x="2" y="2" width="60" height="60" rx="14" ry="14" fill="url(#g)"/>
  <text x="32" y="33" font-family="system-ui, -apple-system, Segoe UI, Roboto, sans-serif" font-size="34" font-weight="600" fill="#ffffff" text-anchor="middle" dominant-baseline="central">${initial}</text>
</svg>
`;
}

// ----------------------------------------------------------------------------
// Real brand logos via Simple Icons (CC0), fetched at BUILD time only.
//
// `https://cdn.simpleicons.org/{slug}` returns a single-path SVG already filled
// with the brand's primary color. We embed that SVG (and the parsed hex color)
// into the catalog so the app can render the logo inline, fully offline.
//
// Most catalog ids map 1:1 to a Simple Icons slug; the overrides below cover the
// cases where they differ. Any service without a match degrades gracefully to
// the local letter-monogram SVG above.
// ----------------------------------------------------------------------------
const SLUG_OVERRIDES = Object.freeze({
  x: "x",
  protonmail: "proton",
  aws: "amazonwebservices",
  "ebay-classifieds": "etsy",
  booking: "bookingdotcom",
  microsoft: "microsoftoutlook", // generic "microsoft" mark isn't published
  socialclub: "rockstargames",
  chatgpt: "openai",
});

function slugFor(service) {
  return SLUG_OVERRIDES[service.id] ?? service.id;
}

/**
 * Fetch a brand SVG + its primary hex color from Simple Icons. Returns null on
 * any failure (offline, 404, unexpected body) so the caller can fall back to a
 * locally generated monogram.
 */
async function fetchBrandIcon(slug) {
  try {
    const res = await fetch(`https://cdn.simpleicons.org/${slug}`, {
      redirect: "follow",
      headers: { Accept: "image/svg+xml,*/*" },
    });
    if (!res.ok) return null;
    const text = (await res.text()).trim();
    if (!text.startsWith("<svg") || !text.includes("</svg>")) return null;
    const m = text.match(/fill="(#[0-9a-fA-F]{6})"/);
    return { svg: text, color: m ? m[1].toLowerCase() : "" };
  } catch {
    return null;
  }
}

// Domains for well-known brands that Simple Icons no longer publishes (trademark
// removals). For these we bundle the site's favicon as a raster data URL — still
// a real, recognizable logo and fetched at build time only.
const DOMAIN_FALLBACK = Object.freeze({
  linkedin: "linkedin.com",
  microsoft: "microsoft.com",
  amazon: "amazon.com",
  slack: "slack.com",
  yahoo: "yahoo.com",
  adobe: "adobe.com",
  aws: "aws.amazon.com",
  heroku: "heroku.com",
  kraken: "kraken.com",
  yandex: "yandex.com",
  // Batch-update services (favicon backup when Simple Icons has no match).
  socialclub: "socialclub.rockstargames.com",
  ea: "ea.com",
  sellix: "sellix.com",
  coinsph: "coins.ph",
  pldt: "pldthome.com",
  rcbc: "rcbc.com",
  okx: "okx.com",
  metamask: "metamask.io",
  chatgpt: "chatgpt.com",
  payoneer: "payoneer.com",
  kiro: "kiro.dev",
  datablitz: "datablitz.com.ph",
  onlinejobs: "onlinejobs.ph",
  threads: "threads.net",
});

/**
 * Fetch a site's favicon as a data URL (build-time only). Returns null on
 * failure so the caller can fall back to a monogram.
 */
const BROWSER_UA =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

/** fetch with an abort timeout so a slow/blocking site can't hang the build. */
async function fetchWithTimeout(url, ms = 9000) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), ms);
  try {
    return await fetch(url, {
      redirect: "follow",
      signal: controller.signal,
      headers: { "User-Agent": BROWSER_UA, Accept: "*/*" },
    });
  } finally {
    clearTimeout(timer);
  }
}

function toDataUrl(buf, contentType) {
  const ct = (contentType || "image/png").split(";")[0];
  return `data:${ct};base64,${buf.toString("base64")}`;
}

/**
 * Pull a HIGH-RES icon straight from the site's HTML (apple-touch-icon, an SVG
 * favicon, or the largest declared icon). This yields crisp logos where favicon
 * proxies only return a tiny, blurry image. Returns a data URL or null.
 */
async function fetchSiteIcon(domain) {
  for (const base of [`https://${domain}/`, `https://www.${domain}/`]) {
    try {
      const res = await fetchWithTimeout(base);
      if (!res.ok) continue;
      const html = await res.text();
      const tags = (html.match(/<link[^>]+>/gi) || []).filter(
        (t) => /rel=["'][^"']*icon/i.test(t) && !/mask-icon/i.test(t),
      );
      const candidates = [];
      for (const tag of tags) {
        const href = (tag.match(/href=["']([^"']+)["']/i) || [])[1];
        if (!href) continue;
        const sizes = (tag.match(/sizes=["']([^"']+)["']/i) || [])[1] || "";
        const isApple = /apple-touch-icon/i.test(tag);
        const isSvg = /\.svg(\?|$)/i.test(href);
        const dim = parseInt((sizes.match(/(\d+)x\d+/) || [])[1] || "0", 10);
        // Prefer crisp vector, then apple-touch, then largest declared size.
        const score = (isSvg ? 1000 : 0) + (isApple ? 200 : 0) + dim;
        try {
          candidates.push({ url: new URL(href, base).href, score });
        } catch {
          /* skip malformed href */
        }
      }
      candidates.sort((a, b) => b.score - a.score);
      for (const cand of candidates.slice(0, 4)) {
        try {
          const ir = await fetchWithTimeout(cand.url);
          if (!ir.ok) continue;
          const ct = ir.headers.get("content-type") || "";
          if (!/image\//i.test(ct)) continue;
          const buf = Buffer.from(await ir.arrayBuffer());
          if (buf.length < 600) continue; // too small / likely blurry
          return toDataUrl(buf, ct);
        } catch {
          /* try next candidate */
        }
      }
    } catch {
      /* try next base */
    }
  }
  return null;
}

/**
 * Best available raster/vector icon for a domain: a crisp icon from the site's
 * own HTML first, then Google's favicon service (256px, then 128px).
 */
async function fetchFavicon(domain) {
  const fromSite = await fetchSiteIcon(domain);
  if (fromSite) return fromSite;

  for (const url of [
    `https://t0.gstatic.com/faviconV2?client=SOCIAL&type=FAVICON&fallback_opts=TYPE,SIZE,URL&url=https://${domain}&size=256`,
    `https://www.google.com/s2/favicons?domain=${domain}&sz=128`,
  ]) {
    try {
      const res = await fetchWithTimeout(url);
      if (!res.ok) continue;
      const buf = Buffer.from(await res.arrayBuffer());
      if (buf.length < 200) continue;
      return toDataUrl(buf, res.headers.get("content-type") || "image/png");
    } catch {
      /* try next source */
    }
  }
  return null;
}

// ----------------------------------------------------------------------------
// Validation — fail loudly if curated data drifts from the documented shape.
// ----------------------------------------------------------------------------
function validate(services) {
  const errors = [];
  const seenIds = new Set();
  const seenIcons = new Set();

  for (const [i, s] of services.entries()) {
    const where = `service[${i}] (${s && s.id ? s.id : "?"})`;
    if (!s.id || typeof s.id !== "string" || s.id !== s.id.toLowerCase()) {
      errors.push(`${where}: "id" must be a non-empty lowercase string`);
    }
    if (seenIds.has(s.id)) errors.push(`${where}: duplicate id "${s.id}"`);
    seenIds.add(s.id);

    if (!s.name || typeof s.name !== "string") {
      errors.push(`${where}: "name" must be a non-empty string`);
    }
    if (!s.icon || typeof s.icon !== "string" || !s.icon.endsWith(".svg")) {
      errors.push(`${where}: "icon" must be a non-empty .svg filename`);
    }
    if (seenIcons.has(s.icon)) errors.push(`${where}: duplicate icon "${s.icon}"`);
    seenIcons.add(s.icon);

    if (!Array.isArray(s.aliases) || s.aliases.some((a) => typeof a !== "string" || a !== a.toLowerCase())) {
      errors.push(`${where}: "aliases" must be an array of lowercase strings`);
    }
    if (!Array.isArray(s.recommended_fields) || s.recommended_fields.length === 0) {
      errors.push(`${where}: "recommended_fields" must be a non-empty array`);
    } else {
      for (const [j, fld] of s.recommended_fields.entries()) {
        const fwhere = `${where}.recommended_fields[${j}]`;
        if (!fld.label || typeof fld.label !== "string") {
          errors.push(`${fwhere}: "label" must be a non-empty string`);
        }
        if (!VALID_TYPES.has(fld.type)) {
          errors.push(`${fwhere}: invalid type "${fld.type}"`);
        }
        if (typeof fld.secret !== "boolean") {
          errors.push(`${fwhere}: "secret" must be a boolean`);
        }
      }
    }
  }

  if (errors.length > 0) {
    throw new Error(`Catalog validation failed:\n  - ${errors.join("\n  - ")}`);
  }
}

// ----------------------------------------------------------------------------
// Main build.
// ----------------------------------------------------------------------------
async function main() {
  // Fetch a real brand logo for each service (build-time only), falling back to
  // a local monogram when there's no match. Sequential to stay gentle on the
  // CDN; this is a manual, one-off build step.
  const services = [];
  let realLogos = 0;
  const fallbacks = [];

  for (const s of SERVICES) {
    const icon = `${s.id}.svg`;

    let svg = "";
    let color = "";
    let iconData = "";
    let fileContent;

    if (LOCAL_ICON_OVERRIDES[s.id]) {
      // Bundled high-quality logo wins over any online fetch.
      iconData = await localIconDataUrl(LOCAL_ICON_OVERRIDES[s.id]);
      fileContent = svgForService({ id: s.id, name: s.name });
      realLogos++;
    } else {
      const fetched = await fetchBrandIcon(slugFor(s));
      if (fetched) {
        svg = fetched.svg;
        color = fetched.color;
        fileContent = fetched.svg;
        realLogos++;
      } else if (DOMAIN_FALLBACK[s.id]) {
        // No Simple Icons match — try the site favicon as a raster logo.
        const data = await fetchFavicon(DOMAIN_FALLBACK[s.id]);
        if (data) {
          iconData = data;
          realLogos++;
        } else {
          fallbacks.push(s.id);
        }
        fileContent = svgForService({ id: s.id, name: s.name });
      } else {
        fileContent = svgForService({ id: s.id, name: s.name });
        fallbacks.push(s.id);
      }
    }

    services.push({
      id: s.id,
      name: s.name,
      icon,
      aliases: s.aliases,
      recommended_fields: s.recommended_fields,
      // Inline brand logo + primary color (SVG), or a raster favicon data URL.
      // All empty → the UI renders a monogram.
      svg,
      color,
      icon_data: iconData,
      // Not serialized into the catalog JSON; only written to the icon file.
      _fileContent: fileContent,
    });
  }

  validate(services);

  // Clean & recreate the icons directory so removed services don't leave orphans.
  await rm(ICONS_DIR, { recursive: true, force: true });
  await mkdir(ICONS_DIR, { recursive: true });

  // Write the catalog JSON (pretty-printed for reviewable diffs), dropping the
  // transient per-service file content.
  const jsonServices = services.map(({ _fileContent, ...rest }) => rest);
  await writeFile(CATALOG_JSON, JSON.stringify(jsonServices, null, 2) + "\n", "utf8");

  // Write one SVG per service (real logo or monogram fallback).
  for (const s of services) {
    await writeFile(join(ICONS_DIR, s.icon), s._fileContent, "utf8");
  }

  console.log(`Keyhaven catalog built (dev-only tooling, not shipped):`);
  console.log(`  • ${services.length} services → ${CATALOG_JSON}`);
  console.log(`  • ${realLogos} real brand logos, ${fallbacks.length} monogram fallbacks`);
  if (fallbacks.length > 0) {
    console.log(`  • fallbacks: ${fallbacks.join(", ")}`);
  }
  console.log(`  • icons → ${ICONS_DIR}`);
}

main().catch((err) => {
  console.error(err.message || err);
  process.exitCode = 1;
});
