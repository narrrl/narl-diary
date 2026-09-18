<script lang="ts">
  import type { NodeSummary } from './api'
  import { hooks, runCommand } from './commands'
  import SpaceBar from './SpaceBar.svelte'
  import { workspace } from './store.svelte'
  import { formatDay, formatTime, formatWeekday } from './util'

  let list = $state<HTMLUListElement>()

  // Keep the cursor line visible while navigating with j/k.
  $effect(() => {
    const index = workspace.cursor
    const node = list?.querySelector<HTMLElement>(`[data-index="${index}"]`)
    node?.scrollIntoView({ block: 'nearest' })
  })

  function label(node: NodeSummary) {
    return node.name.trim() || node.excerpt.trim() || node.slug
  }

  /* A folder is listed by what is in it; a document by when it was written. */
  const aside = (node: NodeSummary) =>
    node.kind === 'document' ? formatTime(node.created_at) : `${node.child_count}`

  function pick(index: number) {
    workspace.cursor = index
    void workspace.guard(() => workspace.openSelected())
  }
</script>

<div class="sidebar">
  <div class="head">
    <span class="accent">~/{workspace.space?.slug ?? 'diary'}</span>
    <span class="faint count">
      {workspace.nodes.length} {workspace.nodes.length === 1 ? 'item' : 'items'}
    </span>
    <span class="tools">
      <button title="new document (o)" onclick={() => runCommand('new')}>+</button>
      <button title="new folder (O)" onclick={() => hooks.openCommandLine(':mkdir ')}>[+]</button>
      <button title="search (/)" onclick={() => hooks.openCommandLine('/')}>/</button>
      <button title="command line (:)" onclick={() => hooks.openCommandLine(':')}>:</button>
      <button title="help (?)" onclick={() => runCommand('help')}>?</button>
    </span>
  </div>

  <SpaceBar />

  {#if workspace.query}
    <div class="filter">
      <span class="faint">/</span>{workspace.query}
      <button title="clear search" onclick={() => workspace.guard(() => workspace.search(''))}>esc</button>
    </div>
  {/if}

  <ul bind:this={list}>
    {#each workspace.nodes as node, index (node.id)}
      {@const previous = workspace.nodes[index - 1]}
      <!-- Day headings only make sense in a list that is a journal: documents,
           in date order, not a search and not a folder listing. -->
      {#if !workspace.query && node.kind === 'document' && (!previous || previous.kind !== 'document' || formatDay(previous.created_at) !== formatDay(node.created_at))}
        <li class="daybreak">
          <span>{formatDay(node.created_at)}</span>
          <span class="faint">{formatWeekday(node.created_at)}</span>
        </li>
      {/if}
      <li>
        <button
          class="row"
          class:active={index === workspace.cursor}
          class:open={workspace.open?.id === node.id}
          data-index={index}
          onclick={() => pick(index)}
        >
          <span class="caret">{index === workspace.cursor ? '>' : ' '}</span>
          <span class="time">{aside(node)}</span>
          <span class="title">{node.kind === 'document' ? '' : '/'}{label(node)}</span>
          {#if node.shared}<span class="shared" title="shared">◉</span>{/if}
        </button>
      </li>
    {:else}
      <li class="empty faint">
        {workspace.query ? 'no matches' : 'nothing here yet — o writes, O makes a folder'}
      </li>
    {/each}
  </ul>
</div>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg-alt);
    border-right: 1px solid var(--line);
    min-width: 0;
  }

  .head,
  .filter {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--line);
    flex: none;
  }

  .filter {
    color: var(--warn);
    overflow: hidden;
    white-space: nowrap;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 4px 0 24px;
    overflow-y: auto;
    flex: 1;
  }

  .daybreak {
    display: flex;
    gap: 8px;
    padding: 10px 10px 2px;
    color: var(--fg-dim);
    font-size: 12px;
    position: sticky;
    top: 0;
    background: var(--bg-alt);
  }

  .row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    width: 100%;
    border: 0;
    border-radius: 0;
    padding: 2px 10px;
    text-align: left;
    color: var(--fg-dim);
  }

  .row:hover { background: var(--bg-lift); color: var(--fg); }
  .row.open { color: var(--fg); }
  .row.active { background: var(--select); color: var(--fg); }

  .caret { color: var(--accent); flex: none; width: 1ch; }
  .time { color: var(--fg-faint); flex: none; font-size: 12px; }
  .title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
  .shared { color: var(--accent); flex: none; }
  .empty { padding: 12px 10px; }

  .count { flex: 1; text-align: right; overflow: hidden; white-space: nowrap; }
  .tools { display: flex; gap: 2px; flex: none; }
  .tools button { padding: 0 6px; line-height: 18px; }

  /* Roomier tap targets where there is no keyboard. */
  @media (pointer: coarse) {
    .row { padding: 7px 10px; }
    .tools button { padding: 4px 10px; }
    .count { display: none; }
  }
</style>
