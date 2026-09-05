<script lang="ts">
  import { untrack } from 'svelte'
  import { completions, runCommand, hooks } from './commands'
  import { diary } from './store.svelte'

  interface Props {
    initial: string
    close: () => void
  }

  const { initial, close }: Props = $props()

  let value = $state(untrack(() => initial))
  let input = $state<HTMLInputElement>()
  const isSearch = $derived(value.startsWith('/'))
  const matches = $derived(isSearch ? [] : completions(value).slice(0, 8))

  $effect(() => {
    input?.focus()
  })

  // A search is live: the list filters as you type, like `incsearch`.
  $effect(() => {
    if (!isSearch) return
    const query = value.slice(1)
    const timer = setTimeout(() => void diary.guard(() => diary.search(query)), 120)
    return () => clearTimeout(timer)
  })

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    const line = value
    close()
    if (isSearch) {
      hooks.focusList()
      return
    }
    await runCommand(line)
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault()
      close()
      hooks.focusList()
    }
    if (event.key === 'Tab' && matches.length > 0) {
      event.preventDefault()
      value = `:${matches[0].name} `
    }
  }
</script>

<div class="cmdline">
  {#if matches.length > 0 && value.replace(/^:/, '').length > 0}
    <ul class="completions">
      {#each matches as spec (spec.name)}
        <li>
          <button onclick={() => (value = `:${spec.name} `, input?.focus())}>
            <span class="accent">:{spec.name}</span>
            {#if spec.args}<span class="faint">{spec.args}</span>{/if}
            <span class="dim">{spec.help}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  <form onsubmit={submit}>
    <input bind:this={input} bind:value {onkeydown} spellcheck="false" autocomplete="off" />
  </form>
</div>

<style>
  .cmdline {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 20;
  }

  form { display: flex; }

  input {
    flex: 1;
    height: 24px;
    background: var(--bg);
    border: 0;
    border-top: 1px solid var(--accent-dim);
    border-radius: 0;
    color: var(--accent);
    padding: 0 8px;
    font-size: 13px;
  }

  .completions {
    list-style: none;
    margin: 0;
    padding: 4px 0;
    background: var(--bg-alt);
    border-top: 1px solid var(--line);
    max-height: 40vh;
    overflow-y: auto;
  }

  .completions button {
    display: flex;
    gap: 8px;
    width: 100%;
    border: 0;
    border-radius: 0;
    text-align: left;
    padding: 1px 10px;
    font-size: 12px;
  }
  .completions button:hover { background: var(--select); }
</style>
