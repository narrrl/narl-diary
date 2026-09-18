<script lang="ts">
  import { commands, groups, type CommandSpec, type Group } from './commands'
  import { workspace } from './store.svelte'

  const keys: [string, string][] = [
    ['j / k · ↓ ↑', 'move down / up the list'],
    ['gg / G', 'first / last row'],
    ['Enter · l', 'open a document, or enter a folder'],
    ['h', 'up one level, or back out of a document'],
    ['o', 'new document here, straight into insert mode'],
    ['O', 'new folder here'],
    ['i · a', 'edit the open document'],
    ['Esc · q', 'back out to the list'],
    ['/', 'full-text search, inside the current space'],
    ['n', 'clear the search'],
    ['s', 'toggle sharing for the document'],
    ['y', 'mint a new share link and copy it'],
    ['x · dd', 'delete what is selected (asks first)'],
    [':', 'command line'],
    ['Ctrl-S', 'save, from anywhere'],
    ['?', 'this screen — in the editor too'],
  ]

  /* The board rebinds most keys, so it gets its own column rather than
     footnotes on the list keys. */
  const boardKeys: [string, string][] = [
    ['h / l', 'to the list left / right'],
    ['j / k', 'down / up the cards'],
    ['J / K', 'move the card down / up its list'],
    ['H / L', 'move the card to the list left / right'],
    ['Enter', 'open the document the card links to'],
    ['t · Space', 'tick the card off, or un-tick it'],
    ['o', 'new card on this list'],
    ['x', 'delete the card (asks first)'],
    ['q · Esc', 'back to the tree'],
  ]

  const titles: Record<Group, string> = {
    tree: 'spaces & folders',
    board: 'boards',
    documents: 'documents',
    editing: 'writing',
    sharing: 'sharing & export',
    view: 'session',
  }

  const grouped = groups.map(
    (group) => [group, commands.filter((spec) => spec.group === group)] as [Group, CommandSpec[]],
  )

  /** `:write(w)` — the name first, then whatever else reaches it. */
  const signature = (spec: CommandSpec) =>
    `:${spec.name}${spec.aliases?.length ? `(${spec.aliases.join(' ')})` : ''}`
</script>

<div class="sheet">
  <header>
    <span class="accent">:help</span>
    <span class="faint">everything is a key or a `:` command</span>
    <button onclick={() => (workspace.overlay = 'none')}>esc</button>
  </header>

  <div class="cols">
    <section class="keys">
      <h2>keys</h2>
      <dl>
        {#each keys as [key, description] (key)}
          <dt>{key}</dt>
          <dd>{description}</dd>
        {/each}
      </dl>
    </section>

    <section class="keys">
      <h2>board keys</h2>
      <dl>
        {#each boardKeys as [key, description] (key)}
          <dt>{key}</dt>
          <dd>{description}</dd>
        {/each}
      </dl>
    </section>

    {#each grouped as [group, specs] (group)}
      <section>
        <h2>{titles[group]}</h2>
        <dl>
          {#each specs as spec (spec.name)}
            <dt>
              {signature(spec)}{#if spec.args}<span class="args faint">{spec.args}</span>{/if}
            </dt>
            <dd>{spec.help}</dd>
            {#if spec.bang}
              <dt>:{spec.name}!</dt>
              <dd>{spec.bang.help}</dd>
            {/if}
          {/each}
        </dl>
      </section>
    {/each}
  </div>

  <footer class="faint">
    inside the editor the full vim keymap is live — hjkl, w/b, dd, ciw, visual mode, macros, and
    <span class="accent">:w</span> to write. drop or paste an image, video or audio file into the
    editor to attach it. <span class="accent">:12</span> jumps straight to entry 12. yanks and
    deletes are copied to the system clipboard —
    <span class="accent">:set noclipboard</span> keeps them in vim's registers.
  </footer>
</div>

<style>
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 14px;
    width: min(100%, 110ch);
    max-height: 100%;
    background: var(--bg-alt);
    border: 1px solid var(--accent-dim);
    border-radius: var(--radius);
    padding: 14px 16px;
    overflow-y: auto;
    overflow-x: hidden;
  }

  header { display: flex; align-items: baseline; gap: 12px; }
  header .faint { flex: 1; font-size: 12px; }
  header button { align-self: center; }

  /* Newspaper columns, so the short groups fill the gaps under the long ones. */
  .cols { columns: 38ch; column-gap: 28px; }

  section { break-inside: avoid; min-width: 0; margin: 0 0 16px; }

  h2 {
    font-size: 12px;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.1em;
    margin: 0 0 6px;
    border-bottom: 1px dashed var(--line);
    padding-bottom: 4px;
  }

  dl {
    margin: 0;
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 3px 12px;
    font-size: 12px;
  }
  dt { color: var(--accent); white-space: nowrap; }
  .args { margin-left: 0.6ch; }
  dd { margin: 0; color: var(--fg-dim); overflow-wrap: anywhere; }

  footer { font-size: 12px; border-top: 1px solid var(--line); padding-top: 10px; }
</style>
