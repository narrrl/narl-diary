<script lang="ts">
  import { runCommand } from './commands'
  import { renderMarkdown } from './markdown'
  import { diary } from './store.svelte'
  import { formatStamp, formatWeekday, isTouchDevice, relative } from './util'

  const entry = $derived(diary.open!)
  const html = $derived(renderMarkdown(diary.editing ? diary.draft.body : entry.body))
  const words = $derived(
    (diary.editing ? diary.draft.body : entry.body).split(/\s+/).filter(Boolean).length,
  )
  const shareUrl = $derived(entry.share_token ? `${location.origin}/s/${entry.share_token}` : null)
</script>

<section class="pane">
  <header>
    <div class="meta">
      <span class="accent">entry:{entry.id}</span>
      <span class="faint">{formatStamp(diary.draft.created_at)} {formatWeekday(diary.draft.created_at)}</span>
      <span class="faint">·</span>
      <span class="faint">edited {relative(entry.updated_at)}</span>
      <span class="faint">·</span>
      <span class="faint">{words} words</span>
      {#if diary.dirty}<span class="warn">[+]</span>{/if}
    </div>

    <div class="actions">
      {#if isTouchDevice}
        <button onclick={() => runCommand('q')}>← list</button>
      {/if}
      {#if diary.editing}
        <button onclick={() => runCommand('w')}>:w</button>
        <button onclick={() => runCommand('upload')}>attach</button>
        <button onclick={() => runCommand('wq')}>done</button>
      {:else}
        <button onclick={() => runCommand('e')}>edit</button>
      {/if}
      <button class:on={entry.shared} onclick={() => runCommand(entry.shared ? 'unshare' : 'share')}>
        {entry.shared ? 'unshare' : 'share'}
      </button>
      <button onclick={() => runCommand('d')}>delete</button>
      <button onclick={() => runCommand('q')}>close</button>
    </div>
  </header>

  {#if shareUrl}
    <div class="sharebar">
      <span class="accent">◉ public</span>
      <a href={shareUrl} target="_blank" rel="noreferrer">{shareUrl}</a>
      <button onclick={() => runCommand('link')}>copy</button>
    </div>
  {/if}

  {#if diary.editing}
    <input
      class="title"
      placeholder="title (optional)"
      bind:value={diary.draft.title}
      oninput={() => (diary.dirty = true)}
      onkeydown={(event) => {
        if (event.key === 'Enter') {
          event.preventDefault()
          document.querySelector<HTMLElement>('.cm-content')?.focus()
        }
      }}
    />
    <div class="body">
      {#key entry.id}
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
    <div class="body reading">
      <article class="md">
        {#if entry.title.trim()}<h1 class="entrytitle">{entry.title}</h1>{/if}
        <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
        {@html html}
        {#if !entry.body.trim()}
          <p class="faint">this entry is empty — press <span class="accent">i</span> to write.</p>
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
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    justify-content: space-between;
    padding: 6px 12px;
    border-bottom: 1px solid var(--line);
    flex: none;
  }

  .meta { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; font-size: 12px; }
  .warn { color: var(--warn); }
  .actions { display: flex; gap: 4px; flex-wrap: wrap; }
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
