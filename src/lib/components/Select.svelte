<script lang="ts" generics="T extends string | number">
  /*
   * Select — a fully custom, accessible dropdown that replaces the native
   * <select>. Native option lists are rendered by the OS and can't be themed
   * with CSS, so this owns both the control and the popup list to match the
   * Keyhaven design tokens everywhere.
   *
   * Accessibility: the trigger is a `combobox` button (aria-haspopup/expanded);
   * the popup is a `listbox` with `option` children, full keyboard support
   * (Up/Down/Home/End/Enter/Esc/Tab) and `aria-activedescendant` tracking. It
   * closes on outside pointer-down and on blur.
   */
  interface Option {
    value: T;
    label: string;
  }

  let {
    value,
    options,
    onChange,
    ariaLabel,
    id,
    disabled = false,
    minWidth = "0",
  }: {
    value: T;
    options: Option[];
    onChange: (value: T) => void;
    ariaLabel?: string;
    id?: string;
    disabled?: boolean;
    /** Minimum width of the control (CSS length), so callers can size it. */
    minWidth?: string;
  } = $props();

  let open = $state(false);
  let activeIndex = $state(-1);
  let buttonEl = $state<HTMLButtonElement>();
  let listEl = $state<HTMLUListElement>();

  // Stable base for option element ids (used by aria-activedescendant).
  const uid = `kh-select-${Math.random().toString(36).slice(2, 9)}`;

  const selectedIndex = $derived(options.findIndex((o) => o.value === value));
  const selectedLabel = $derived(
    selectedIndex >= 0 ? options[selectedIndex].label : "",
  );
  const activeId = $derived(
    open && activeIndex >= 0 ? `${uid}-opt-${activeIndex}` : undefined,
  );

  function openList(): void {
    if (disabled) return;
    open = true;
    activeIndex = selectedIndex >= 0 ? selectedIndex : 0;
    queueMicrotask(() => listEl?.focus());
  }

  function closeList(focusButton = true): void {
    open = false;
    if (focusButton) buttonEl?.focus();
  }

  function toggle(): void {
    open ? closeList() : openList();
  }

  function choose(index: number): void {
    const opt = options[index];
    if (!opt) return;
    if (opt.value !== value) onChange(opt.value);
    closeList();
  }

  function onButtonKeydown(event: KeyboardEvent): void {
    if (["ArrowDown", "ArrowUp", "Enter", " "].includes(event.key)) {
      event.preventDefault();
      openList();
    }
  }

  function onListKeydown(event: KeyboardEvent): void {
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        activeIndex = Math.min(options.length - 1, activeIndex + 1);
        break;
      case "ArrowUp":
        event.preventDefault();
        activeIndex = Math.max(0, activeIndex - 1);
        break;
      case "Home":
        event.preventDefault();
        activeIndex = 0;
        break;
      case "End":
        event.preventDefault();
        activeIndex = options.length - 1;
        break;
      case "Enter":
      case " ":
        event.preventDefault();
        choose(activeIndex);
        break;
      case "Escape":
        event.preventDefault();
        closeList();
        break;
      case "Tab":
        closeList(false);
        break;
    }
  }

  function onWindowPointerDown(event: PointerEvent): void {
    if (!open) return;
    const target = event.target as Node;
    if (buttonEl?.contains(target) || listEl?.contains(target)) return;
    open = false;
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} />

<div class="kh-select" class:open style="min-width:{minWidth}">
  <button
    bind:this={buttonEl}
    {id}
    type="button"
    class="trigger"
    role="combobox"
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-label={ariaLabel}
    aria-controls={`${uid}-list`}
    {disabled}
    onclick={toggle}
    onkeydown={onButtonKeydown}
  >
    <span class="trigger-label">{selectedLabel}</span>
    <svg
      class="chevron"
      class:flipped={open}
      viewBox="0 0 20 20"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      stroke-width="1.8"
      stroke-linecap="round"
      stroke-linejoin="round"
    >
      <path d="M5 7.5l5 5 5-5" />
    </svg>
  </button>

  {#if open}
    <ul
      bind:this={listEl}
      id={`${uid}-list`}
      class="list"
      role="listbox"
      tabindex="-1"
      aria-activedescendant={activeId}
      aria-label={ariaLabel}
      onkeydown={onListKeydown}
    >
      {#each options as opt, i (opt.value)}
        <li
          id={`${uid}-opt-${i}`}
          role="option"
          aria-selected={opt.value === value}
          class:active={i === activeIndex}
          class:selected={opt.value === value}
          onpointerenter={() => (activeIndex = i)}
          onpointerdown={(e) => {
            e.preventDefault();
            choose(i);
          }}
        >
          <span class="opt-label">{opt.label}</span>
          {#if opt.value === value}
            <svg
              class="check"
              viewBox="0 0 20 20"
              aria-hidden="true"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M4 10.5l4 4 8-9" />
            </svg>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .kh-select {
    position: relative;
    display: inline-flex;
  }

  .trigger {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--kh-space-2);
    width: 100%;
    padding: var(--kh-space-2) var(--kh-space-3);
    background-color: var(--kh-surface);
    border: 1px solid var(--kh-border-strong);
    border-radius: var(--kh-radius-sm);
    color: var(--kh-text);
    font: inherit;
    font-size: var(--kh-font-size-sm);
    font-weight: var(--kh-font-weight-medium);
    cursor: pointer;
    text-align: left;
    transition:
      border-color var(--kh-motion-fast) var(--kh-ease),
      box-shadow var(--kh-motion-fast) var(--kh-ease),
      background-color var(--kh-motion-fast) var(--kh-ease);
  }

  .trigger:hover:not(:disabled) {
    border-color: var(--kh-accent);
    background-color: var(--kh-surface-sunken);
  }

  .trigger:focus-visible {
    outline: none;
    border-color: var(--kh-accent);
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  .open .trigger {
    border-color: var(--kh-accent);
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  .trigger-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chevron {
    flex: 0 0 auto;
    width: 16px;
    height: 16px;
    color: var(--kh-text-muted);
    transition: transform var(--kh-motion-fast) var(--kh-ease);
  }

  .chevron.flipped {
    transform: rotate(180deg);
  }

  .list {
    position: absolute;
    z-index: 50;
    top: calc(100% + 4px);
    left: 0;
    min-width: 100%;
    width: max-content;
    max-width: 280px;
    max-height: 280px;
    overflow-y: auto;
    margin: 0;
    padding: var(--kh-space-1);
    list-style: none;
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius);
    box-shadow: var(--kh-shadow-lg);
    outline: none;
  }

  .list li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--kh-space-3);
    padding: var(--kh-space-2) var(--kh-space-3);
    border-radius: var(--kh-radius-sm);
    font-size: var(--kh-font-size-sm);
    color: var(--kh-text);
    cursor: pointer;
    white-space: nowrap;
  }

  .list li.active {
    background: var(--kh-accent-subtle);
    color: var(--kh-accent-hover);
  }

  .list li.selected {
    font-weight: var(--kh-font-weight-semibold);
  }

  .check {
    flex: 0 0 auto;
    width: 15px;
    height: 15px;
    color: var(--kh-accent);
  }

  .opt-label {
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
