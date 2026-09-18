<script lang="ts">
  /*
   * The board of the current space: columns of cards, in the same terminal
   * idiom as the list. The keys live in App.svelte with every other key, so
   * this file only draws and — for a pointer — moves the two cursors.
   */
  import type { Card } from './api'
  import { hooks, runCommand } from './commands'
  import SpaceBar from './SpaceBar.svelte'
  import { workspace } from './store.svelte'
  import { formatDay } from './util'

  let columns = $state<HTMLDivElement>()

  // Keep the selected card on screen while walking the board with the keyboard.
  $effect(() => {
    const at = `${workspace.listCursor}:${workspace.cardCursor}`
    columns?.querySelector<HTMLElement>(`[data-at="${at}"]`)?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  })

  const midnight = () => {
    const today = new Date()
    today.setHours(0, 0, 0, 0)
    return Math.floor(today.getTime() / 1000)
  }

  /** `overdue`, `today` or nothing — a date only earns colour when it bites. */
  function urgency(card: Card): '' | 'today' | 'overdue' {
    if (card.due_at === null || card.done_at !== null) return ''
    const start = midnight()
    if (card.due_at < start) return 'overdue'
    return card.due_at < start + 86400 ? 'today' : ''
  }

  function pick(list: number, card: number) {
    workspace.listCursor = list
    workspace.cardCursor = card
  }
</script>

<div class="board">
  <div class="head">
    <span class="accent">~/{workspace.space?.slug ?? ''}/board</span>
    <span class="faint count">
      {workspace.board?.lists.reduce((n, list) => n + list.cards.length, 0) ?? 0} cards
    </span>
    <span class="tools">
      <button title="new card (o)" onclick={() => hooks.openCommandLine(':card ')}>+</button>
      <button title="new list" onclick={() => hooks.openCommandLine(':list ')}>[+]</button>
      <button title="back to the tree (:board)" onclick={() => runCommand('board')}>tree</button>
      <button title="command line (:)" onclick={() => hooks.openCommandLine(':')}>:</button>
      <button title="help (?)" onclick={() => runCommand('help')}>?</button>
    </span>
  </div>

  <SpaceBar />

  <div class="columns" bind:this={columns}>
    {#each workspace.board?.lists ?? [] as list, listIndex (list.id)}
      <section class="column" class:on={listIndex === workspace.listCursor}>
        <header>
          <span class="name">{list.name}</span>
          <span class="faint">{list.cards.length}</span>
        </header>

        <ul>
          {#each list.cards as card, cardIndex (card.id)}
            <li>
              <button
                class="card"
                class:active={listIndex === workspace.listCursor && cardIndex === workspace.cardCursor}
                class:done={card.done_at !== null}
                data-at="{listIndex}:{cardIndex}"
                onclick={() => pick(listIndex, cardIndex)}
                ondblclick={() => workspace.guard(() => workspace.openCard())}
              >
                <span class="title">
                  <span class="tick">{card.done_at === null ? '[ ]' : '[x]'}</span>
                  {card.title}
                </span>
                {#if card.due_at !== null || card.node_id !== null}
                  <span class="meta faint">
                    {#if card.due_at !== null}
                      <span class={urgency(card)}>due {formatDay(card.due_at)}</span>
                    {/if}
                    {#if card.node_id !== null}
                      <span class="doc" title="Enter opens it">→ {card.node_name ?? `#${card.node_id}`}</span>
                    {/if}
                  </span>
                {/if}
              </button>
            </li>
          {:else}
            <li class="empty faint">—</li>
          {/each}
        </ul>
      </section>
    {:else}
      <div class="empty faint">no lists — :list &lt;name&gt; makes one</div>
    {/each}
  </div>
</div>

<style>
  .board {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--line);
    flex: none;
  }

  .count { flex: 1; text-align: right; white-space: nowrap; }
  .tools { display: flex; gap: 2px; flex: none; }
  .tools button { padding: 0 6px; line-height: 18px; }

  .columns {
    flex: 1;
    min-height: 0;
    display: flex;
    align-items: stretch;
    gap: 0;
    overflow: auto;
  }

  .column {
    display: flex;
    flex-direction: column;
    min-width: 26ch;
    flex: 1 1 0;
    max-width: 44ch;
    border-right: 1px solid var(--line);
    min-height: 0;
  }

  .column header {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--line);
    color: var(--fg-dim);
    position: sticky;
    top: 0;
    background: var(--bg);
  }

  /* The list the cursor is on, marked the way the sidebar marks its space. */
  .column.on header .name { color: var(--accent); font-weight: 700; }
  .column.on { background: var(--bg-alt); }

  ul {
    list-style: none;
    margin: 0;
    padding: 4px 0 24px;
    overflow-y: auto;
    flex: 1;
  }

  .card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    width: 100%;
    border: 0;
    border-radius: 0;
    padding: 4px 10px;
    text-align: left;
    color: var(--fg-dim);
    white-space: normal;
  }

  .card:hover { background: var(--bg-lift); color: var(--fg); }
  .card.active { background: var(--select); color: var(--fg); }
  .card.done .title { text-decoration: line-through; color: var(--fg-faint); }

  .tick { color: var(--accent); }
  .title { display: block; }
  .meta { display: flex; gap: 8px; font-size: 12px; padding-left: 4ch; }
  .meta .today { color: var(--warn); }
  .meta .overdue { color: var(--error); }
  .doc { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 24ch; }

  .empty { padding: 12px 10px; }

  @media (pointer: coarse) {
    .card { padding: 8px 10px; }
    .tools button { padding: 4px 10px; }
  }
</style>
