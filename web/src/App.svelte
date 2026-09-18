<script lang="ts">
  import { hooks, runCommand } from './lib/commands'
  import { embedSnippet } from './lib/markdown'
  import Board from './lib/Board.svelte'
  import CommandLine from './lib/CommandLine.svelte'
  import DocumentPane from './lib/DocumentPane.svelte'
  import Help from './lib/Help.svelte'
  import Login from './lib/Login.svelte'
  import MediaBrowser from './lib/MediaBrowser.svelte'
  import Shared from './lib/Shared.svelte'
  import Sidebar from './lib/Sidebar.svelte'
  import StatusBar from './lib/StatusBar.svelte'
  import { workspace } from './lib/store.svelte'

  /*
   * A share link is /s/<token>#<key>. The fragment never leaves the browser, so
   * a crawler that only scraped the path cannot ask the server for the entry.
   */
  const shareToken = location.pathname.startsWith('/s/')
    ? decodeURIComponent(location.pathname.slice(3))
    : null
  const shareKey = shareToken ? decodeURIComponent(location.hash.slice(1)) : ''

  let cmdline = $state<string | null>(null)
  let fileInput = $state<HTMLInputElement>()
  let folderInput = $state<HTMLInputElement>()
  let archiveInput = $state<HTMLInputElement>()
  let shell = $state<HTMLDivElement>()
  let overlayEl = $state<HTMLDivElement>()
  let pending = $state('')

  if (!shareToken) void workspace.boot()

  $effect(() => {
    document.documentElement.dataset.theme = workspace.theme
  })

  $effect(() => {
    folderInput?.setAttribute('webkitdirectory', '')
  })

  $effect(() => {
    hooks.pickFiles = () => fileInput?.click()
    hooks.pickFolder = () => folderInput?.click()
    hooks.pickArchive = () => archiveInput?.click()
    hooks.focusList = () => shell?.focus()
    hooks.openCommandLine = (initial) => (cmdline = initial)
  })

  /*
   * The overlay takes focus while it is up, so its keys (q, ?, esc) reach the
   * window handler instead of being swallowed by the editor underneath.
   */
  $effect(() => {
    if (workspace.overlay !== 'none') {
      overlayEl?.focus()
      return () => (workspace.editing ? hooks.focusEditor() : shell?.focus())
    }
  })

  // Guard against losing a half-written entry to a stray refresh.
  $effect(() => {
    if (!workspace.dirty) return
    const warn = (event: BeforeUnloadEvent) => event.preventDefault()
    window.addEventListener('beforeunload', warn)
    return () => window.removeEventListener('beforeunload', warn)
  })

  async function attach(event: Event) {
    const input = event.target as HTMLInputElement
    const files = Array.from(input.files ?? [])
    input.value = ''
    if (files.length === 0) return
    await workspace.guard(async () => {
      const uploaded = await workspace.upload(files)
      hooks.insertText(uploaded.map(embedSnippet).join('\n\n'))
    })
  }

  /*
   * A folder upload is the same multipart body as any other, except that each
   * file carries the path it had inside the folder — which is what makes the
   * tree on the other side possible.
   */
  async function importFolder(event: Event) {
    const input = event.target as HTMLInputElement
    const files = Array.from(input.files ?? [])
    input.value = ''
    if (files.length === 0) return
    await workspace.guard(() => workspace.importFolder(files))
  }

  /*
   * The board reads the same keys as the list, one dimension wider: h/l walk
   * the lists, j/k the cards, and the shifted pair takes the card along. Only
   * `:` and `?` fall through to the keys below, because everything else would
   * act on a tree that is not on screen.
   */
  function boardKey(key: string): boolean {
    switch (key) {
      case 'h':
      case 'ArrowLeft':
        workspace.moveList(-1)
        return true
      case 'l':
      case 'ArrowRight':
        workspace.moveList(1)
        return true
      case 'j':
      case 'ArrowDown':
        workspace.moveCard(1)
        return true
      case 'k':
      case 'ArrowUp':
        workspace.moveCard(-1)
        return true
      case 'J':
        void workspace.guard(() => workspace.nudgeCard(1))
        return true
      case 'K':
        void workspace.guard(() => workspace.nudgeCard(-1))
        return true
      case 'H':
        void workspace.guard(() => workspace.sendCard(-1))
        return true
      case 'L':
        void workspace.guard(() => workspace.sendCard(1))
        return true
      case 'G':
        workspace.cardCursor = Math.max((workspace.list?.cards.length ?? 0) - 1, 0)
        return true
      case 'Enter':
        void workspace.guard(() => workspace.openCard())
        return true
      case 'o':
        cmdline = ':card '
        return true
      case 't':
      case ' ':
        void workspace.guard(() => workspace.toggleDone())
        return true
      case 'x':
        void runCommand('delete')
        return true
      case 'r':
        void workspace.guard(() => workspace.reloadBoard())
        return true
      case 'q':
      case 'Escape':
        workspace.closeBoard()
        return true
      default:
        return false
    }
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
      if (workspace.open) void workspace.guard(() => workspace.save())
      return
    }

    if (event.key === 'Escape') {
      if (workspace.overlay !== 'none') {
        workspace.overlay = 'none'
        return
      }
      if (cmdline !== null) {
        cmdline = null
        shell?.focus()
        return
      }
    }

    if (workspace.overlay !== 'none') {
      if (event.key === 'q' || event.key === '?') {
        event.preventDefault()
        workspace.overlay = 'none'
      }
      return
    }

    if (cmdline !== null || isTypingTarget(event.target)) return
    if (event.ctrlKey || event.metaKey || event.altKey) return

    const key = event.key
    const previous = pending
    pending = ''

    if (workspace.view === 'board') {
      if (previous === 'g' && key === 'g') {
        event.preventDefault()
        workspace.cardCursor = 0
        return
      }
      if (key === 'g') {
        pending = key
        return
      }
      if (boardKey(key)) {
        event.preventDefault()
        return
      }
    }

    // Two-key sequences: gg and dd.
    if (previous === 'g' && key === 'g') {
      event.preventDefault()
      workspace.cursor = 0
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
        workspace.move(1)
        break
      case 'k':
      case 'ArrowUp':
        event.preventDefault()
        workspace.move(-1)
        break
      case 'G':
        event.preventDefault()
        workspace.cursor = Math.max(workspace.nodes.length - 1, 0)
        break
      case 'Enter':
      case 'l':
      case 'ArrowRight':
        event.preventDefault()
        void workspace.guard(() => workspace.openSelected())
        break
      case 'o':
        event.preventDefault()
        void runCommand('new')
        break
      case 'O':
        // The name has to come from somewhere, and the command line is where
        // names are typed — this only saves typing `:mkdir `.
        event.preventDefault()
        cmdline = ':mkdir '
        break
      case 'i':
      case 'a':
        event.preventDefault()
        void runCommand('edit')
        break
      case 'h':
      case 'ArrowLeft':
        // Out of the document if one is open, otherwise up a level — the same
        // key means "leftwards" at both depths.
        event.preventDefault()
        if (workspace.open) void runCommand('q')
        else void workspace.guard(() => workspace.up())
        break
      case 'q':
      case 'Escape':
        event.preventDefault()
        if (workspace.open) void runCommand('q')
        break
      case 'x':
        event.preventDefault()
        void runCommand('d')
        break
      case 's':
        event.preventDefault()
        void runCommand(workspace.selected?.shared ? 'unshare' : 'share')
        break
      case 'y':
        event.preventDefault()
        void runCommand('link')
        break
      case 'n':
        event.preventDefault()
        if (workspace.query) void workspace.guard(() => workspace.search(''))
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
  <Shared token={shareToken} shareKey={shareKey} />
{:else if workspace.booting}
  <div class="boot faint">booting ~/workspace…</div>
{:else if !workspace.user}
  <Login />
{:else}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="shell" bind:this={shell} tabindex="-1">
    {#if workspace.view === 'board'}
      <!-- The board takes the whole width: its columns are the two panes. -->
      <div class="panes board"><Board /></div>
    {:else}
    <div class="panes" class:reading={!!workspace.open}>
      <div class="list"><Sidebar /></div>
      <div class="entry">
        {#if workspace.open}
          <DocumentPane />
        {:else}
          <div class="placeholder faint">
            <pre>{`  ┌─────────────────────────────┐
  │  nothing open               │
  │                             │
  │  o   write a new document   │
  │  O   make a folder          │
  │  j/k browse, Enter opens    │
  │  h   up one level           │
  │  /   search this space      │
  │  ?   help                   │
  └─────────────────────────────┘`}</pre>
        </div>
        {/if}
      </div>
    </div>
    {/if}

    <StatusBar />

    {#if cmdline !== null}
      <CommandLine initial={cmdline} close={() => (cmdline = null)} />
    {/if}

    {#if workspace.overlay !== 'none'}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions, a11y_no_noninteractive_tabindex -->
      <div class="overlay" tabindex="-1" bind:this={overlayEl} onclick={(e) => e.target === e.currentTarget && (workspace.overlay = 'none')}
      >
        {#if workspace.overlay === 'help'}<Help />{:else}<MediaBrowser />{/if}
      </div>
    {/if}
  </div>
{/if}

<input class="hidden" type="file" multiple bind:this={fileInput} onchange={attach} />
<!-- `webkitdirectory` is set from script: it is the one attribute browsers
     agree on for picking a folder, and no typed attribute list carries it. -->
<input class="hidden" type="file" multiple bind:this={folderInput} onchange={importFolder} />
<input class="hidden" type="file" accept=".zip" bind:this={archiveInput} onchange={importFolder} />

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

  .panes.board { grid-template-columns: 1fr; }

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
