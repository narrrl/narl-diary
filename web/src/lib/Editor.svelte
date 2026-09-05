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

  const editorTheme = EditorView.theme({
    '&': { color: 'var(--fg)', backgroundColor: 'transparent', height: '100%' },
    '.cm-content': { padding: '12px 16px', maxWidth: '90ch' },
    '.cm-line': { padding: '0' },
  })

  /** Route every `:command` typed inside the editor through the shared handler. */
  function bridgeExCommands() {
    for (const spec of commands) {
      const names = [spec.name, ...(spec.aliases ?? [])]
      for (const name of names) {
        if (name.includes('!')) continue
        Vim.defineEx(name, name, (_cm: unknown, params: { argString?: string }) => {
          const arg = (params.argString ?? '').trim()
          // vim parses `:q!` as command `q` with the bang left in the arguments.
          void runCommand(arg === '!' ? `${spec.name}!` : `${spec.name} ${arg}`)
        })
      }
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
          diary.dirty = true
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
