<script lang="ts">
  import { diary } from './store.svelte'
  import { isTouchDevice } from './util'

  const mode = $derived(
    diary.editing ? (diary.vimEnabled ? diary.vimMode.toUpperCase() : 'EDIT') : 'BROWSE',
  )

  const file = $derived(
    diary.open ? `entry:${diary.open.id}${diary.dirty ? ' [+]' : ''}` : '[no entry]',
  )

  const lines = $derived(diary.editing ? diary.draft.body.split('\n').length : null)
</script>

<footer>
  <span class="mode" class:insert={mode === 'INSERT'} class:visual={mode.startsWith('VISUAL')}>
    {mode}
  </span>
  <span class="file">{file}</span>

  <span class="msg" class:error={diary.flash?.kind === 'error'}>
    {diary.flash?.text ?? ''}
  </span>

  {#if lines !== null}<span class="faint">{lines}L</span>{/if}
  <span class="faint hint">
    {isTouchDevice ? 'tap : for commands' : '? help · : command · / search'}
  </span>
  <span class="user accent">{diary.user}</span>
</footer>

<style>
  footer {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 8px 0 0;
    height: 24px;
    background: var(--bg-lift);
    border-top: 1px solid var(--line);
    font-size: 12px;
    flex: none;
    white-space: nowrap;
    overflow: hidden;
  }

  .mode {
    background: var(--accent-dim);
    color: var(--bg);
    font-weight: 700;
    padding: 0 10px;
    height: 100%;
    display: flex;
    align-items: center;
    letter-spacing: 0.08em;
  }
  .mode.insert { background: var(--accent); }
  .mode.visual { background: var(--warn); }

  .file { color: var(--fg-dim); flex: none; }
  .msg { flex: 1; overflow: hidden; text-overflow: ellipsis; color: var(--fg); }
  .msg.error { color: var(--error); }
  .user { flex: none; }

  @media (max-width: 720px) {
    .hint { display: none; }
  }
</style>
