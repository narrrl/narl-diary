<script lang="ts">
  import { commands } from './commands'
  import { diary } from './store.svelte'

  const keys: [string, string][] = [
    ['j / k · ↓ ↑', 'move down / up the entry list'],
    ['gg / G', 'first / last entry'],
    ['Enter · l', 'open the highlighted entry'],
    ['o', 'new entry, straight into insert mode'],
    ['i · a', 'edit the open entry'],
    ['Esc · h · q', 'back out to the list'],
    ['/', 'full-text search'],
    ['n', 'clear the search'],
    ['s', 'toggle sharing for the entry'],
    ['y', 'copy the share link'],
    ['x · dd', 'delete the entry (asks first)'],
    [': ', 'command line'],
    ['Ctrl-S', 'save, from anywhere'],
    ['?', 'this screen'],
  ]
</script>

<div class="sheet">
  <header>
    <span class="accent">:help</span>
    <button onclick={() => (diary.overlay = 'none')}>esc</button>
  </header>

  <div class="cols">
    <section>
      <h2>keys</h2>
      <dl>
        {#each keys as [key, description] (key)}
          <dt>{key}</dt>
          <dd>{description}</dd>
        {/each}
      </dl>
    </section>

    <section>
      <h2>commands</h2>
      <dl>
        {#each commands as spec (spec.name)}
          <dt>
            :{spec.name}{#if spec.aliases?.length}<span class="faint"> ({spec.aliases.join(' ')})</span>{/if}
            {#if spec.args}<span class="faint"> {spec.args}</span>{/if}
          </dt>
          <dd>{spec.help}</dd>
          {#if spec.bang}
            <dt>:{spec.name}!{#if spec.aliases?.length}<span class="faint"> ({spec.aliases.map((a) => `${a}!`).join(' ')})</span>{/if}</dt>
            <dd>{spec.bang.help}</dd>
          {/if}
        {/each}
      </dl>
    </section>
  </div>

  <footer class="faint">
    inside the editor the full vim keymap is live — hjkl, w/b, dd, ciw, visual mode, macros, and
    <span class="accent">:w</span> to write. drop or paste an image, video or audio file into the
    editor to attach it.
  </footer>
</div>

<style>
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 12px;
    width: min(100%, 90ch);
    max-height: 100%;
    background: var(--bg-alt);
    border: 1px solid var(--accent-dim);
    border-radius: var(--radius);
    padding: 14px 16px;
    overflow-y: auto;
  }

  header { display: flex; justify-content: space-between; align-items: center; }

  .cols { display: grid; grid-template-columns: repeat(auto-fit, minmax(30ch, 1fr)); gap: 20px; }

  h2 { font-size: 12px; color: var(--fg-dim); text-transform: uppercase; letter-spacing: 0.1em; margin: 0 0 6px; }

  dl { margin: 0; display: grid; grid-template-columns: auto 1fr; gap: 2px 12px; font-size: 12px; }
  dt { color: var(--accent); white-space: nowrap; }
  dd { margin: 0; color: var(--fg-dim); }

  footer { font-size: 12px; border-top: 1px solid var(--line); padding-top: 10px; }
</style>
