<script lang="ts">
  import { api } from './api'
  import { hooks } from './commands'
  import { embedSnippet } from './markdown'
  import { workspace } from './store.svelte'
  import { formatBytes, formatStamp } from './util'

  const usage = (nodeIds: number[]) =>
    nodeIds.length === 0 ? 'unattached' : nodeIds.map((id) => `#${id}`).join(' ')

  async function remove(id: string, filename: string, nodeIds: number[]) {
    const used = nodeIds.length === 1 ? '1 document' : `${nodeIds.length} documents`
    const warning = nodeIds.length > 0 ? ` it is embedded in ${used}, which will break.` : ''
    if (!confirm(`delete ${filename}?${warning}`)) return
    await workspace.guard(async () => {
      await api.deleteMedia(id)
      await workspace.loadMedia()
    })
  }
</script>

<div class="sheet">
  <header>
    <span class="accent">:media</span>
    <span class="faint">{workspace.media.length} files</span>
    <button onclick={() => (workspace.overlay = 'none')}>esc</button>
  </header>

  <ul>
    {#each workspace.media as file (file.id)}
      <li>
        <div class="thumb">
          {#if file.mime.startsWith('image/')}
            <img src={file.url} alt={file.filename} loading="lazy" />
          {:else}
            <span class="faint">{file.mime.split('/')[0]}</span>
          {/if}
        </div>

        <div class="info">
          <span class="name">{file.filename}</span>
          <span class="faint">
            {formatBytes(file.size)} · {formatStamp(file.created_at)} ·
            {usage(file.node_ids)}
          </span>
        </div>

        <div class="actions">
          {#if workspace.editing}
            <button
              onclick={() => {
                hooks.insertText(embedSnippet(file))
                workspace.overlay = 'none'
              }}>insert</button
            >
          {/if}
          <button onclick={() => workspace.copy(location.origin + file.url)}>copy url</button>
          <button onclick={() => remove(file.id, file.filename, file.node_ids)}>rm</button>
        </div>
      </li>
    {:else}
      <li class="faint">nothing uploaded yet — attach a file from the editor.</li>
    {/each}
  </ul>
</div>

<style>
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: min(100%, 90ch);
    max-height: 100%;
    background: var(--bg-alt);
    border: 1px solid var(--accent-dim);
    border-radius: var(--radius);
    padding: 14px 16px;
    overflow: hidden;
  }

  header { display: flex; justify-content: space-between; align-items: center; gap: 12px; }

  ul { list-style: none; margin: 0; padding: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }

  li { display: flex; align-items: center; gap: 12px; padding: 4px; border-bottom: 1px solid var(--line); }

  .thumb {
    width: 48px;
    height: 36px;
    flex: none;
    display: grid;
    place-items: center;
    background: var(--bg);
    border: 1px solid var(--line);
    overflow: hidden;
    font-size: 10px;
  }
  .thumb img { width: 100%; height: 100%; object-fit: cover; }

  .info { display: flex; flex-direction: column; min-width: 0; flex: 1; font-size: 12px; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .actions { display: flex; gap: 4px; flex: none; }
</style>
