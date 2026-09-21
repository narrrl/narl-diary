<script lang="ts">
  import { hooks, runCommand, type ScrollAmount } from './commands'
  import { renderMarkdown } from './markdown'
  import { workspace } from './store.svelte'
  import { formatStamp, formatWeekday, isTouchDevice, relative } from './util'

  const doc = $derived(workspace.open!)
  const html = $derived(renderMarkdown(workspace.editing ? workspace.draft.body : doc.body))
  const words = $derived(
    (workspace.editing ? workspace.draft.body : doc.body).split(/\s+/).filter(Boolean).length,
  )
  /*
   * Only the token half of a share link is recoverable — the key lives in the
   * fragment, and the server kept nothing but its hash — so the bar shows the
   * link's shape and offers a fresh one rather than a copy of the old.
   */
  const sharePath = $derived(doc.share_token ? `${location.origin}/s/${doc.share_token}#…` : null)

  let reader = $state<HTMLElement>()

  /*
   * j/k reach this pane through the window handler in App.svelte, which has no
   * way to know how tall a line is here — so the pane does the arithmetic and
   * says whether it scrolled anything at all.
   */
  $effect(() => {
    hooks.scrollReader = (amount: ScrollAmount) => {
      if (!reader) return false
      const line = parseFloat(getComputedStyle(reader).lineHeight) || 24
      const half = reader.clientHeight / 2
      switch (amount) {
        case 'top':
          reader.scrollTo({ top: 0 })
          break
        case 'bottom':
          reader.scrollTo({ top: reader.scrollHeight })
          break
        case 'halfdown':
          reader.scrollBy({ top: half })
          break
        case 'halfup':
          reader.scrollBy({ top: -half })
          break
        default:
          reader.scrollBy({ top: amount * line * 3 })
      }
      return true
    }
    return () => (hooks.scrollReader = () => false)
  })

  /**
   * `/n/<space>/<slug>` links — the ones the importer writes between imported
   * documents — are followed in place rather than reloading the application.
   */
  function follow(event: MouseEvent) {
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) return
    const anchor = (event.target as HTMLElement | null)?.closest('a')
    const href = anchor?.getAttribute('href') ?? ''
    if (!href.startsWith('/n/')) return
    event.preventDefault()
    const path = href.slice(3).split('#')[0]
    void workspace.guard(() => workspace.openPath(decodeURIComponent(path)))
  }
</script>

<section class="pane">
  <header>
    <div class="crumb" title="{workspace.here}/{doc.slug}">
      <span class="accent">{workspace.here}/{doc.slug}</span>
      {#if workspace.dirty}<span class="warn">[+]</span>{/if}
    </div>

    <div class="row">
      <div class="meta">
        <span class="faint">{formatStamp(workspace.draft.created_at)} {formatWeekday(workspace.draft.created_at)}</span>
        <span class="faint">·</span>
        <span class="faint">edited {relative(doc.updated_at)}</span>
        <span class="faint">·</span>
        <span class="faint">{words} words</span>
      </div>

      <div class="actions">
        {#if isTouchDevice}
          <button onclick={() => runCommand('q')}>← list</button>
        {/if}
        {#if workspace.editing}
          <button onclick={() => runCommand('w')}>:w</button>
          <button onclick={() => runCommand('upload')}>attach</button>
          <button onclick={() => runCommand('wq')}>done</button>
        {:else}
          <button onclick={() => runCommand('e')}>edit</button>
        {/if}
        <button class:on={doc.shared} onclick={() => runCommand(doc.shared ? 'unshare' : 'share')}>
          {doc.shared ? 'unshare' : 'share'}
        </button>
        <button onclick={() => runCommand('d')}>delete</button>
        <button onclick={() => runCommand('q')}>close</button>
      </div>
    </div>
  </header>

  {#if sharePath}
    <div class="sharebar">
      <span class="accent">◉ public</span>
      <span class="faint">{sharePath}</span>
      <button onclick={() => runCommand('link')}>new link</button>
    </div>
  {/if}

  {#if workspace.editing}
    <input
      class="title"
      placeholder="name (optional)"
      bind:value={workspace.draft.name}
      oninput={() => workspace.touch()}
      onkeydown={(event) => {
        if (event.key === 'Enter') {
          event.preventDefault()
          document.querySelector<HTMLElement>('.cm-content')?.focus()
        }
      }}
    />
    <div class="body">
      {#key doc.id}
        {#await import('./Editor.svelte')}
          <div class="loading faint">loading editor…</div>
        {:then module}
          {@const Editor = module.default}
          <Editor />
        {/await}
      {/key}
    </div>
    {#if isTouchDevice}
      <div class="mobilebar">
        <button onclick={() => runCommand('upload')}>+ media</button>
        <button onclick={() => runCommand('w')}>save</button>
        <button onclick={() => runCommand('wq')}>done</button>
      </div>
    {/if}
  {:else}
    <div class="body reading" bind:this={reader}>
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
      <article class="md" onclick={follow}>
        {#if doc.name.trim()}<h1 class="entrytitle">{doc.name}</h1>{/if}
        <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
        {@html html}
        {#if !doc.body.trim()}
          <p class="faint">this document is empty — press <span class="accent">i</span> to write.</p>
        {/if}
      </article>
    </div>
  {/if}
</section>

<style>
  .pane {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
  }

  header {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--line);
    flex: none;
  }

  .crumb {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    justify-content: space-between;
    min-width: 0;
  }

  .meta { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; font-size: 12px; min-width: 0; }
  .warn { color: var(--warn); flex: none; }
  .actions { display: flex; gap: 4px; flex-wrap: wrap; flex: none; }
  .actions .on { color: var(--accent); border-color: var(--accent-dim); }

  .sharebar {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 4px 12px;
    background: var(--bg-lift);
    border-bottom: 1px solid var(--line);
    font-size: 12px;
    overflow-x: auto;
    white-space: nowrap;
    flex: none;
  }

  .title {
    flex: none;
    margin: 8px 12px 0;
    background: transparent;
    border: 0;
    border-bottom: 1px dashed var(--line);
    border-radius: 0;
    color: var(--accent);
    font-weight: 700;
    padding: 2px 4px;
  }

  .loading { padding: 16px 20px; }

  .body { flex: 1; min-height: 0; overflow: hidden; }
  .reading { overflow-y: auto; padding: 16px 20px 30vh; }
  .entrytitle { margin-top: 0 !important; }
  .entrytitle::before { content: none !important; }

  .mobilebar {
    display: flex;
    gap: 6px;
    padding: 6px 12px calc(6px + env(safe-area-inset-bottom));
    border-top: 1px solid var(--line);
    background: var(--bg-alt);
    flex: none;
  }
  .mobilebar button { flex: 1; padding: 8px; }
</style>
