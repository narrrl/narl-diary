<script lang="ts">
  import type { EntrySummary } from './api'
  import { hooks, runCommand } from './commands'
  import { diary } from './store.svelte'
  import { formatDay, formatTime, formatWeekday } from './util'

  let list = $state<HTMLUListElement>()

  // Keep the cursor line visible while navigating with j/k.
  $effect(() => {
    const index = diary.cursor
    const node = list?.querySelector<HTMLElement>(`[data-index="${index}"]`)
    node?.scrollIntoView({ block: 'nearest' })
  })

  function label(entry: EntrySummary) {
    return entry.title.trim() || entry.excerpt.trim() || '(empty)'
  }

  function pick(index: number) {
    diary.cursor = index
    void diary.guard(() => diary.openSelected())
  }
</script>

<div class="sidebar">
  <div class="head">
    <span class="accent">~/diary</span>
    <span class="faint count">
      {diary.entries.length} {diary.entries.length === 1 ? 'entry' : 'entries'}
    </span>
    <span class="tools">
      <button title="new entry (o)" onclick={() => runCommand('new')}>+</button>
      <button title="search (/)" onclick={() => hooks.openCommandLine('/')}>/</button>
      <button title="command line (:)" onclick={() => hooks.openCommandLine(':')}>:</button>
      <button title="help (?)" onclick={() => runCommand('help')}>?</button>
    </span>
  </div>

  {#if diary.query}
    <div class="filter">
      <span class="faint">/</span>{diary.query}
      <button title="clear search" onclick={() => diary.guard(() => diary.search(''))}>esc</button>
    </div>
  {/if}

  <ul bind:this={list}>
    {#each diary.entries as entry, index (entry.id)}
      {@const previous = diary.entries[index - 1]}
      {#if !diary.query && (!previous || formatDay(previous.created_at) !== formatDay(entry.created_at))}
        <li class="daybreak">
          <span>{formatDay(entry.created_at)}</span>
          <span class="faint">{formatWeekday(entry.created_at)}</span>
        </li>
      {/if}
      <li>
        <button
          class="row"
          class:active={index === diary.cursor}
          class:open={diary.open?.id === entry.id}
          data-index={index}
          onclick={() => pick(index)}
        >
          <span class="caret">{index === diary.cursor ? '>' : ' '}</span>
          <span class="time">{formatTime(entry.created_at)}</span>
          <span class="title">{label(entry)}</span>
          {#if entry.shared}<span class="shared" title="shared">◉</span>{/if}
        </button>
      </li>
    {:else}
      <li class="empty faint">
        {diary.query ? 'no matches' : 'no entries yet — press o to write one'}
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
