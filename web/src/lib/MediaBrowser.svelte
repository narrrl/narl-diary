<script lang="ts">
  import { api } from './api'
  import { hooks } from './commands'
  import { embedSnippet } from './markdown'
  import { diary } from './store.svelte'
  import { formatBytes, formatStamp } from './util'

  const usage = (entryIds: number[]) =>
    entryIds.length === 0 ? 'unattached' : entryIds.map((id) => `entry:${id}`).join(' ')

  async function remove(id: string, filename: string, entryIds: number[]) {
    const used = entryIds.length === 1 ? '1 entry' : `${entryIds.length} entries`
    const warning = entryIds.length > 0 ? ` it is embedded in ${used}, which will break.` : ''
    if (!confirm(`delete ${filename}?${warning}`)) return
    await diary.guard(async () => {
      await api.deleteMedia(id)
      await diary.loadMedia()
    })
  }
</script>

<div class="sheet">
  <header>
    <span class="accent">:media</span>
    <span class="faint">{diary.media.length} files</span>
    <button onclick={() => (diary.overlay = 'none')}>esc</button>
  </header>

  <ul>
    {#each diary.media as file (file.id)}
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
            {usage(file.entry_ids)}
          </span>
        </div>

        <div class="actions">
          {#if diary.editing}
            <button
              onclick={() => {
                hooks.insertText(embedSnippet(file))
                diary.overlay = 'none'
              }}>insert</button
            >
          {/if}
          <button onclick={() => diary.copy(location.origin + file.url)}>copy url</button>
          <button onclick={() => remove(file.id, file.filename, file.entry_ids)}>rm</button>
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
