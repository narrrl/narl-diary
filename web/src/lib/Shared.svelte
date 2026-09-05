<script lang="ts">
  import { api, type SharedEntry } from './api'
  import { renderMarkdown } from './markdown'
  import { formatStamp, formatWeekday } from './util'

  const { token }: { token: string } = $props()

  let entry = $state<SharedEntry | null>(null)
  let error = $state('')

  $effect(() => {
    api
      .readShared(token)
      .then((data) => {
        entry = data
        document.title = data.title.trim() || `~/diary ${formatStamp(data.created_at)}`
      })
      .catch(() => (error = 'this link is not valid, or the entry is no longer shared.'))
  })
</script>

<main>
  {#if error}
    <p class="error">{error}</p>
  {:else if entry}
    <article class="md">
      <div class="meta faint">
        {formatStamp(entry.created_at)} {formatWeekday(entry.created_at)}
        <span class="tag">shared entry</span>
      </div>
      {#if entry.title.trim()}<h1>{entry.title}</h1>{/if}
      <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
      {@html renderMarkdown(entry.body, token)}
    </article>
    <footer class="faint">— written in ~/diary</footer>
  {:else}
    <p class="faint">loading…</p>
  {/if}
</main>

<style>
  main {
    height: 100%;
    overflow-y: auto;
    padding: 40px 20px 20vh;
    display: flex;
    flex-direction: column;
    align-items: center;
  }

  article { width: min(100%, 78ch); }

  .meta { display: flex; gap: 10px; align-items: center; font-size: 12px; margin-bottom: 12px; }

  .tag {
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0 6px;
    color: var(--accent-dim);
  }

  h1 { margin-top: 0 !important; }
  h1::before { content: none !important; }

  footer { width: min(100%, 78ch); margin-top: 40px; font-size: 12px; }
  p { width: min(100%, 78ch); }
</style>
