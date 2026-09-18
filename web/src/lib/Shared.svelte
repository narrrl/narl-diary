<script lang="ts">
  import { api, type SharedDocument } from './api'
  import { renderMarkdown } from './markdown'
  import { formatStamp, formatWeekday } from './util'

  const { token, shareKey }: { token: string; shareKey: string } = $props()

  let doc = $state<SharedDocument | null>(null)
  let error = $state('')

  $effect(() => {
    if (!shareKey) {
      error = 'this link is incomplete — the part after # is missing from it.'
      return
    }
    api
      .readShared(token, shareKey)
      .then((data) => {
        doc = data
        document.title = data.name.trim() || `~/workspace ${formatStamp(data.created_at)}`
      })
      .catch(() => (error = 'this link is not valid, or the document is no longer shared.'))
  })
</script>

<main>
  {#if error}
    <p class="error">{error}</p>
  {:else if doc}
    <article class="md">
      <div class="meta faint">
        {formatStamp(doc.created_at)} {formatWeekday(doc.created_at)}
        <span class="tag">shared document</span>
      </div>
      {#if doc.name.trim()}<h1>{doc.name}</h1>{/if}
      <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
      {@html renderMarkdown(doc.body, { token, key: shareKey })}
    </article>
    <footer class="faint">— written in ~/workspace</footer>
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
