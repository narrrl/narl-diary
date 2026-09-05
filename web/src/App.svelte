<script lang="ts">
  import { hooks, runCommand } from './lib/commands'
  import { embedSnippet } from './lib/markdown'
  import CommandLine from './lib/CommandLine.svelte'
  import EntryPane from './lib/EntryPane.svelte'
  import Help from './lib/Help.svelte'
  import Login from './lib/Login.svelte'
  import MediaBrowser from './lib/MediaBrowser.svelte'
  import Shared from './lib/Shared.svelte'
  import Sidebar from './lib/Sidebar.svelte'
  import StatusBar from './lib/StatusBar.svelte'
  import { diary } from './lib/store.svelte'

  const shareToken = location.pathname.startsWith('/s/')
    ? decodeURIComponent(location.pathname.slice(3))
    : null

  let cmdline = $state<string | null>(null)
  let fileInput = $state<HTMLInputElement>()
  let shell = $state<HTMLDivElement>()
  let pending = $state('')

  if (!shareToken) void diary.boot()

  $effect(() => {
    document.documentElement.dataset.theme = diary.theme
  })

  $effect(() => {
    hooks.pickFiles = () => fileInput?.click()
    hooks.focusList = () => shell?.focus()
    hooks.openCommandLine = (initial) => (cmdline = initial)
  })

  // Guard against losing a half-written entry to a stray refresh.
  $effect(() => {
    if (!diary.dirty) return
    const warn = (event: BeforeUnloadEvent) => event.preventDefault()
    window.addEventListener('beforeunload', warn)
    return () => window.removeEventListener('beforeunload', warn)
  })

  async function attach(event: Event) {
    const input = event.target as HTMLInputElement
    const files = Array.from(input.files ?? [])
    input.value = ''
    if (files.length === 0) return
    await diary.guard(async () => {
      const uploaded = await diary.upload(files)
      hooks.insertText(uploaded.map(embedSnippet).join('\n\n'))
    })
  }

  function isTypingTarget(target: EventTarget | null) {
    const node = target as HTMLElement | null
    return (
      !!node &&
      (node.tagName === 'INPUT' ||
        node.tagName === 'TEXTAREA' ||
        node.isContentEditable ||
        !!node.closest?.('.cm-editor'))
    )
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 's' && (event.metaKey || event.ctrlKey)) {
      event.preventDefault()
      if (diary.open) void diary.guard(() => diary.save())
      return
    }

    if (event.key === 'Escape') {
      if (diary.overlay !== 'none') {
        diary.overlay = 'none'
        return
      }
      if (cmdline !== null) {
        cmdline = null
        shell?.focus()
        return
      }
    }

    if (cmdline !== null || isTypingTarget(event.target)) return
    if (event.ctrlKey || event.metaKey || event.altKey) return

    const key = event.key
    const previous = pending
    pending = ''

    if (diary.overlay !== 'none') {
      if (key === 'q' || key === '?') diary.overlay = 'none'
      return
    }

    // Two-key sequences: gg and dd.
    if (previous === 'g' && key === 'g') {
      event.preventDefault()
      diary.cursor = 0
      return
    }
    if (previous === 'd' && key === 'd') {
      event.preventDefault()
      void runCommand('d')
      return
    }
    if (key === 'g' || key === 'd') {
      pending = key
      return
    }

    switch (key) {
      case 'j':
      case 'ArrowDown':
        event.preventDefault()
        diary.move(1)
        break
      case 'k':
      case 'ArrowUp':
        event.preventDefault()
        diary.move(-1)
        break
      case 'G':
        event.preventDefault()
        diary.cursor = Math.max(diary.entries.length - 1, 0)
        break
      case 'Enter':
      case 'l':
      case 'ArrowRight':
        event.preventDefault()
        void diary.guard(() => diary.openSelected())
        break
      case 'o':
        event.preventDefault()
        void runCommand('new')
        break
      case 'i':
      case 'a':
        event.preventDefault()
        void runCommand('edit')
        break
      case 'h':
      case 'ArrowLeft':
      case 'q':
      case 'Escape':
        event.preventDefault()
        if (diary.open) void runCommand('q')
        break
      case 'x':
        event.preventDefault()
        void runCommand('d')
        break
      case 's':
        event.preventDefault()
        void runCommand(diary.selected?.shared ? 'unshare' : 'share')
        break
      case 'y':
        event.preventDefault()
        void runCommand('link')
        break
      case 'n':
        event.preventDefault()
        if (diary.query) void diary.guard(() => diary.search(''))
        break
      case '/':
        event.preventDefault()
        cmdline = '/'
        break
      case ':':
        event.preventDefault()
        cmdline = ':'
        break
      case '?':
        event.preventDefault()
        void runCommand('help')
        break
    }
  }
</script>

<svelte:window {onkeydown} />

{#if shareToken}
  <Shared token={shareToken} />
{:else if diary.booting}
  <div class="boot faint">booting ~/diary…</div>
{:else if !diary.user}
  <Login />
{:else}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="shell" bind:this={shell} tabindex="-1">
    <div class="panes" class:reading={!!diary.open}>
      <div class="list"><Sidebar /></div>
      <div class="entry">
        {#if diary.open}
          <EntryPane />
        {:else}
          <div class="placeholder faint">
            <pre>{`  ┌─────────────────────────────┐
  │  nothing open               │
  │                             │
  │  o   write a new entry      │
  │  j/k browse, Enter opens    │
  │  /   search everything      │
  │  ?   help                   │
  └─────────────────────────────┘`}</pre>
        </div>
        {/if}
      </div>
    </div>

    <StatusBar />

    {#if cmdline !== null}
      <CommandLine initial={cmdline} close={() => (cmdline = null)} />
    {/if}

    {#if diary.overlay !== 'none'}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="overlay" onclick={(e) => e.target === e.currentTarget && (diary.overlay = 'none')}>
        {#if diary.overlay === 'help'}<Help />{:else}<MediaBrowser />{/if}
      </div>
    {/if}
  </div>
{/if}

<input class="hidden" type="file" multiple bind:this={fileInput} onchange={attach} />

<style>
  .boot { display: grid; place-items: center; height: 100%; }

  .shell {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    outline: none;
  }

  .panes {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(28ch, 34ch) 1fr;
  }

  .list, .entry { min-width: 0; overflow: hidden; }

  .placeholder { display: grid; place-items: center; height: 100%; }
  .placeholder pre { margin: 0; font-size: 12px; line-height: 1.5; }

  .overlay {
    position: absolute;
    inset: 0;
    z-index: 30;
    display: grid;
    place-items: center;
    padding: 24px;
    background: color-mix(in srgb, var(--bg) 86%, transparent);
  }

  .hidden { display: none; }

  @media (max-width: 760px) {
    .panes { grid-template-columns: 1fr; }
    .panes .entry { display: none; }
    .panes.reading .list { display: none; }
    .panes.reading .entry { display: block; }
  }
</style>
