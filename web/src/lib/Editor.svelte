<script lang="ts" module>
  /* The vim register controller is global, so it is wrapped once per page. */
  let clipboardMirrored = false
</script>

<script lang="ts">
  import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
  import { markdown, markdownLanguage } from '@codemirror/lang-markdown'
  import { HighlightStyle, indentOnInput, syntaxHighlighting } from '@codemirror/language'
  import { EditorState } from '@codemirror/state'
  import {
    EditorView,
    drawSelection,
    dropCursor,
    highlightActiveLine,
    keymap,
    placeholder,
  } from '@codemirror/view'
  import { tags } from '@lezer/highlight'
  import { Vim, getCM, vim } from '@replit/codemirror-vim'
  import { onDestroy, onMount } from 'svelte'
  import { commands, hooks, runCommand } from './commands'
  import { embedSnippet } from './markdown'
  import { diary } from './store.svelte'

  let host = $state<HTMLDivElement>()
  let view: EditorView | undefined

  const markdownHighlight = HighlightStyle.define([
    { tag: tags.heading, color: 'var(--accent)', fontWeight: '700' },
    { tag: tags.strong, color: 'var(--fg)', fontWeight: '700' },
    { tag: tags.emphasis, color: 'var(--fg)', fontStyle: 'italic' },
    { tag: tags.link, color: 'var(--accent)' },
    { tag: tags.url, color: 'var(--accent-dim)' },
    { tag: tags.monospace, color: 'var(--warn)' },
    { tag: tags.quote, color: 'var(--fg-dim)', fontStyle: 'italic' },
    { tag: tags.list, color: 'var(--accent-dim)' },
    { tag: tags.contentSeparator, color: 'var(--fg-faint)' },
    { tag: tags.processingInstruction, color: 'var(--fg-faint)' },
  ])

  /*
   * CodeMirror's base theme paints panels and the caret for a light editor
   * (`&light .cm-panels { color: black }`), and those selectors outrank
   * anything the app stylesheet can say. Theme rules do win, so the parts that
   * would otherwise render black on black live here rather than in app.css.
   * `dark: true` also stops the light defaults applying in the first place.
   */
  const editorTheme = EditorView.theme(
    {
      '&': { color: 'var(--fg)', backgroundColor: 'transparent', height: '100%' },
      '.cm-content': { padding: '12px 16px', maxWidth: '90ch' },
      '.cm-line': { padding: '0' },
      '.cm-cursor, .cm-dropCursor': { borderLeft: '2px solid var(--accent)' },
      '.cm-panels': {
        backgroundColor: 'var(--bg-alt)',
        color: 'var(--fg)',
        borderColor: 'var(--line)',
      },
      '.cm-panels-bottom': { borderTop: '1px solid var(--line)' },
      /*
       * The `:` prompt is a bare text node in the panel; the input follows it.
       * The input is an inline-block whose baseline sits below the text node's,
       * so the two are laid out as flex items instead of aligned by baseline.
       */
      '.cm-vim-panel': {
        padding: '2px 8px',
        color: 'var(--accent)',
        display: 'flex',
        alignItems: 'center',
      },
      '.cm-vim-panel input': {
        fontFamily: 'inherit',
        fontSize: 'inherit',
        lineHeight: 'inherit',
        color: 'var(--accent)',
        caretColor: 'var(--accent)',
        flex: '1',
      },
    },
    { dark: true },
  )

  /** Route every `:command` typed inside the editor through the shared handler. */
  function bridgeExCommands() {
    for (const spec of commands) {
      const names = [spec.name, ...(spec.aliases ?? [])]
      for (const name of names) {
        Vim.defineEx(name, name, (_cm: unknown, params: { argString?: string }) => {
          const arg = (params.argString ?? '').trim()
          // vim parses `:q!` as command `q` with the bang left in the arguments,
          // so put it back on the name before handing the line over.
          const bang = arg.startsWith('!')
          void runCommand(`${spec.name}${bang ? '!' : ''} ${bang ? arg.slice(1).trim() : arg}`)
        })
      }
    }
  }

  /**
   * `set clipboard=unnamed`, in effect: every yank or delete that lands in the
   * unnamed register is copied out to the system clipboard too. An explicit
   * register (`"ay`) is left alone, and `"+p` still pastes the clipboard back.
   */
  function mirrorUnnamedRegisterToClipboard() {
    if (clipboardMirrored) return
    const registers = Vim.getRegisterController?.()
    if (!registers) return
    clipboardMirrored = true
    const push = registers.pushText.bind(registers)
    registers.pushText = (name, operator, text, linewise, blockwise) => {
      push(name, operator, text, linewise, blockwise)
      if (!name && text && diary.clipboard) void diary.copy(text)
    }
  }

  /**
   * `?` is vim's search-backwards, but the diary advertises it as the help key
   * everywhere else, so it opens the reference here too. `/` still searches.
   */
  function bindHelpKey() {
    Vim.defineAction('diaryHelp', () => void runCommand('help'))
    for (const context of ['normal', 'visual'] as const) {
      Vim.mapCommand('?', 'action', 'diaryHelp', {}, { context })
    }
  }

  /**
   * Media goes on its own line after the one the cursor is on, so attaching a
   * file never cuts a sentence in half.
   */
  function insertAtCursor(text: string) {
    if (!view) return
    const line = view.state.doc.lineAt(view.state.selection.main.head)
    const blank = line.text.trim() === ''
    const at = blank ? line.from : line.to
    const insert = blank ? `${text}\n` : `\n\n${text}\n`
    view.dispatch({
      changes: { from: at, to: blank ? line.to : at, insert },
      selection: { anchor: at + insert.length },
      scrollIntoView: true,
    })
    view.focus()
  }

  async function uploadFiles(files: File[]) {
    if (files.length === 0) return
    await diary.guard(async () => {
      const uploaded = await diary.upload(files)
      insertAtCursor(uploaded.map(embedSnippet).join('\n\n'))
    })
  }

  onMount(() => {
    bridgeExCommands()
    bindHelpKey()
    mirrorUnnamedRegisterToClipboard()

    const extensions = [
      ...(diary.vimEnabled ? [vim({ status: false })] : []),
      history(),
      drawSelection(),
      dropCursor(),
      indentOnInput(),
      highlightActiveLine(),
      EditorState.allowMultipleSelections.of(true),
      markdown({ base: markdownLanguage, codeLanguages: [] }),
      syntaxHighlighting(markdownHighlight),
      EditorView.lineWrapping,
      placeholder('dear diary…'),
      editorTheme,
      keymap.of([
        {
          key: 'Mod-s',
          preventDefault: true,
          run: () => (void diary.guard(() => diary.save()), true),
        },
        ...defaultKeymap,
        ...historyKeymap,
        indentWithTab,
      ]),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          diary.draft.body = update.state.doc.toString()
          diary.touch()
        }
      }),
      EditorView.domEventHandlers({
        paste: (event) => {
          const files = Array.from(event.clipboardData?.files ?? [])
          if (files.length === 0) return false
          event.preventDefault()
          void uploadFiles(files)
          return true
        },
        drop: (event) => {
          const files = Array.from(event.dataTransfer?.files ?? [])
          if (files.length === 0) return false
          event.preventDefault()
          void uploadFiles(files)
          return true
        },
      }),
    ]

    view = new EditorView({
      state: EditorState.create({ doc: diary.draft.body, extensions }),
      parent: host!,
    })

    hooks.focusEditor = () => view?.focus()
    hooks.insertText = insertAtCursor
    view.focus()

    const cm = diary.vimEnabled ? getCM(view) : null
    if (cm) {
      // Keep the status line's mode indicator in step with the editor.
      const emitter = cm as unknown as {
        on?: (event: string, handler: (e: { mode: string }) => void) => void
      }
      emitter.on?.('vim-mode-change', (event) => {
        diary.vimMode = event.mode
      })

      // A fresh entry starts in insert mode; an existing one starts in normal
      // mode, the way opening a file in vim does.
      if (diary.enterInsert) {
        diary.enterInsert = false
        Vim.handleKey(cm, 'i', 'diary')
      }
    }
  })

  onDestroy(() => {
    view?.destroy()
    view = undefined
    hooks.focusEditor = () => {}
    hooks.insertText = () => {}
    diary.vimMode = 'normal'
  })
</script>

<div class="editor" bind:this={host}></div>

<style>
  .editor {
    height: 100%;
    overflow: hidden;
  }
</style>
