<script lang="ts">
  /*
   * ServiceIcon — the one place that decides how a service is visually
   * identified. In priority order it renders:
   *   1. a user-supplied image (custom service data icon) — `imgSrc`
   *   2. a bundled inline brand SVG (Simple Icons) — `svg` (+ `color` tint)
   *   3. a bundled raster favicon data URL — `iconData`
   *   4. a calm letter monogram derived from the name/id — the always-available
   *      fallback
   *
   * The inline `svg` comes from our own build-time catalog (trusted), so
   * rendering it with `{@html}` is safe here.
   */
  let {
    name,
    id = "",
    svg = "",
    color = "",
    iconData = "",
    imgSrc = "",
    size = 36,
  }: {
    name: string;
    id?: string;
    svg?: string;
    color?: string;
    iconData?: string;
    imgSrc?: string;
    size?: number;
  } = $props();

  function hueOf(s: string): number {
    let h = 0;
    for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) % 360;
    return h;
  }

  function monogramOf(n: string): string {
    return (n.trim()[0] ?? "?").toUpperCase();
  }

  const hue = $derived(hueOf(id || name));
  // Subtle brand-colored tile tint when we know the color (8-digit hex alpha).
  const tileBg = $derived(
    /^#[0-9a-fA-F]{6}$/.test(color) ? `${color}1f` : "var(--kh-surface-sunken)",
  );
  const dim = $derived(`${size}px`);
  const monoFont = $derived(`${Math.round(size * 0.42)}px`);
</script>

{#if imgSrc}
  <img
    class="svc-icon img"
    src={imgSrc}
    alt=""
    aria-hidden="true"
    style="width:{dim};height:{dim}"
  />
{:else if svg}
  <span
    class="svc-icon logo"
    style="width:{dim};height:{dim};background:{tileBg}"
    aria-hidden="true">{@html svg}</span
  >
{:else if iconData}
  <img
    class="svc-icon img"
    src={iconData}
    alt=""
    aria-hidden="true"
    style="width:{dim};height:{dim}"
  />
{:else}
  <span
    class="svc-icon mono"
    style="--avatar-hue:{hue};width:{dim};height:{dim};font-size:{monoFont}"
    aria-hidden="true"
  >
    {monogramOf(name)}
  </span>
{/if}

<style>
  .svc-icon {
    flex: 0 0 auto;
    display: grid;
    place-items: center;
    border-radius: var(--kh-radius-pill);
    overflow: hidden;
  }

  .svc-icon.mono {
    font-weight: var(--kh-font-weight-semibold);
    background: hsl(var(--avatar-hue) 45% 93%);
    color: hsl(var(--avatar-hue) 40% 38%);
  }

  .svc-icon.img {
    object-fit: cover;
    background: var(--kh-surface-sunken);
  }

  /* Inset the inline logo within its tile. */
  .svc-icon.logo :global(svg) {
    width: 58%;
    height: 58%;
  }
</style>
