<script lang="ts">
  import { diary } from './store.svelte'

  let username = $state('')
  let password = $state('')
  let error = $state('')
  let busy = $state(false)
  let field = $state<HTMLInputElement>()

  $effect(() => {
    field?.focus()
  })

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    busy = true
    error = ''
    try {
      await diary.login(username, password)
    } catch (e) {
      error = e instanceof Error ? e.message : 'login failed'
      password = ''
    } finally {
      busy = false
    }
  }
</script>

<main>
  <form onsubmit={submit}>
    <pre class="banner">{`╭──────────────────────────────────╮
│  ~/diary                         │
╰──────────────────────────────────╯`}</pre>

    <p class="faint">a private log. one user. one machine.</p>

    <label>
      <span class="accent">login:</span>
      <input bind:this={field} bind:value={username} autocomplete="username" spellcheck="false" />
    </label>

    <label>
      <span class="accent">password:</span>
      <input type="password" bind:value={password} autocomplete="current-password" />
    </label>

    <div class="row">
      <button type="submit" disabled={busy}>{busy ? 'authenticating…' : 'enter'}</button>
      {#if error}<span class="error">{error}</span>{/if}
    </div>
  </form>
</main>

<style>
  main {
    display: grid;
    place-items: center;
    height: 100%;
    padding: 24px;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    width: min(100%, 46ch);
    background: var(--bg-alt);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 20px;
  }

  .banner {
    color: var(--accent);
    font-size: 13px;
    line-height: 1.3;
    margin: 0;
    overflow-x: auto;
  }

  p { margin: 0 0 4px; font-size: 12px; }

  label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; }

  .row { display: flex; align-items: center; gap: 12px; }
  .row button { padding: 6px 16px; }
</style>
