import { diary, type Theme } from './store.svelte'
import { formatDay, parseDay } from './util'

/** Filled in by App.svelte so ex-commands can reach the DOM-bound bits. */
export const hooks = {
  pickFiles: () => {},
  focusEditor: () => {},
  focusList: () => {},
  insertText: (_text: string) => {},
  openCommandLine: (_initial: string) => {},
}

/** Help groups the command list under these headings, in this order. */
export const groups = ['entries', 'editing', 'sharing', 'view'] as const
export type Group = (typeof groups)[number]

export interface CommandSpec {
  name: string
  aliases?: string[]
  args?: string
  group: Group
  help: string
  run: (arg: string) => void | Promise<void>
  /** The forcing variant, `:name!` — vim hands the bang over as an argument. */
  bang?: { help: string; run: (arg: string) => void | Promise<void> }
}

const requireOpen = () => {
  if (!diary.open) {
    diary.say('no entry open', 'error')
    return null
  }
  return diary.open
}

/** Move the cursor and follow it, so `:next` reads as well as `j` then `Enter`. */
const jumpTo = async (index: number) => {
  if (diary.entries.length === 0) return diary.say('no entries', 'error')
  diary.cursor = Math.min(Math.max(index, 0), diary.entries.length - 1)
  await diary.guard(() => diary.openSelected())
}

const themes: Theme[] = ['mocha', 'green', 'amber', 'ice']

const setTheme = (name: string) => {
  if (!themes.includes(name as Theme)) {
    return diary.say(`unknown theme: ${name} — try ${themes.join(', ')}`, 'error')
  }
  diary.setTheme(name as Theme)
  diary.say(`theme ${name}`)
}

/** A filename that survives a download folder: `2026-09-05-first-light.md`. */
const exportName = (title: string, at: number) => {
  const slug = title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')
  return `${formatDay(at)}${slug ? `-${slug}` : ''}.md`
}

export const commands: CommandSpec[] = [
  {
    name: 'write',
    aliases: ['w'],
    group: 'editing',
    help: 'save the open entry',
    run: () => diary.guard(() => diary.save()),
    bang: { help: 'same as :w — the bang is accepted out of habit', run: () => diary.guard(() => diary.save()) },
  },
  {
    name: 'wq',
    aliases: ['x'],
    group: 'editing',
    help: 'save and leave insert/editor mode',
    run: async () => {
      await diary.guard(() => diary.save())
      diary.editing = false
      hooks.focusList()
    },
    bang: {
      help: 'same as :wq — the bang is accepted out of habit',
      run: async () => {
        await diary.guard(() => diary.save())
        diary.editing = false
        hooks.focusList()
      },
    },
  },
  {
    name: 'quit',
    aliases: ['q'],
    group: 'editing',
    help: 'close the editor, or the entry when already reading',
    run: () => {
      if (diary.dirty) {
        diary.say('unsaved changes — :w to write, :q! to discard', 'error')
        return
      }
      if (diary.editing) diary.editing = false
      else {
        diary.open = null
        diary.pane = 'list'
      }
      hooks.focusList()
    },
    bang: {
      help: 'discard changes and close the editor',
      run: async () => {
        const open = diary.open
        diary.editing = false
        diary.dirty = false
        if (open) await diary.guard(() => diary.openEntry(open.id))
        hooks.focusList()
      },
    },
  },
  {
    name: 'new',
    aliases: ['n', 'o'],
    args: '[yyyy-mm-dd]',
    group: 'entries',
    help: 'start a new entry, optionally dated',
    run: async (arg) => {
      await diary.guard(async () => {
        await diary.createEntry()
        if (arg) {
          const at = parseDay(arg)
          if (at === null) return diary.say(`not a date: ${arg}`, 'error')
          diary.draft.created_at = at
          diary.dirty = true
          await diary.save()
        }
      })
      hooks.focusEditor()
    },
  },
  {
    name: 'today',
    aliases: ['t'],
    group: 'entries',
    help: "open today's entry, starting one if there is none yet",
    run: async () => {
      await diary.guard(async () => {
        if (diary.query) await diary.search('')
        const midnight = new Date()
        midnight.setHours(0, 0, 0, 0)
        const from = Math.floor(midnight.getTime() / 1000)
        const entry = diary.entries.find((e) => e.created_at >= from && e.created_at < from + 86400)
        if (entry) await diary.openEntry(entry.id, true)
        else await diary.createEntry()
      })
      hooks.focusEditor()
    },
  },
  {
    name: 'edit',
    aliases: ['e'],
    args: '[id]',
    group: 'editing',
    help: 'edit the open entry, or open an entry by id',
    run: async (arg) => {
      const id = arg ? Number(arg) : diary.open?.id ?? diary.selected?.id
      if (!id || Number.isNaN(id)) return diary.say('usage: :e <id>', 'error')
      await diary.guard(() => diary.openEntry(id, true))
      hooks.focusEditor()
    },
    bang: {
      help: 'throw away the draft and re-read the entry from the server',
      run: async () => {
        const open = requireOpen()
        if (!open) return
        diary.dirty = false
        await diary.guard(() => diary.openEntry(open.id, diary.editing))
        diary.say(`entry:${open.id} reloaded`)
      },
    },
  },
  {
    name: 'next',
    aliases: ['bn'],
    group: 'entries',
    help: 'open the next entry down the list',
    run: () => jumpTo(diary.cursor + 1),
  },
  {
    name: 'prev',
    aliases: ['bp'],
    group: 'entries',
    help: 'open the previous entry',
    run: () => jumpTo(diary.cursor - 1),
  },
  {
    name: 'first',
    group: 'entries',
    help: 'open the newest entry',
    run: () => jumpTo(0),
  },
  {
    name: 'last',
    group: 'entries',
    help: 'open the oldest entry',
    run: () => jumpTo(diary.entries.length - 1),
  },
  {
    name: 'random',
    group: 'entries',
    help: 'open an entry at random — good for re-reading',
    run: () => jumpTo(Math.floor(Math.random() * diary.entries.length)),
  },
  {
    name: 'delete',
    aliases: ['d', 'rm'],
    args: '[id]',
    group: 'entries',
    help: 'delete an entry (asks first)',
    run: async (arg) => {
      const id = arg ? Number(arg) : diary.open?.id ?? diary.selected?.id
      if (!id || Number.isNaN(id)) return diary.say('nothing to delete', 'error')
      if (!confirm(`delete entry #${id}? this cannot be undone.`)) return
      await diary.guard(() => diary.deleteEntry(id))
    },
  },
  {
    name: 'title',
    args: '<text>',
    group: 'editing',
    help: 'set the title of the open entry',
    run: async (arg) => {
      if (!requireOpen()) return
      diary.draft.title = arg
      diary.dirty = true
      await diary.guard(() => diary.save())
    },
  },
  {
    name: 'date',
    args: '<yyyy-mm-dd>',
    group: 'editing',
    help: 're-date the open entry',
    run: async (arg) => {
      if (!requireOpen()) return
      const at = parseDay(arg)
      if (at === null) return diary.say('usage: :date 2026-09-05', 'error')
      diary.draft.created_at = at
      diary.dirty = true
      await diary.guard(() => diary.save())
    },
  },
  {
    name: 'share',
    group: 'sharing',
    help: 'publish the entry behind an unguessable link and copy it',
    run: async () => {
      const id = diary.open?.id ?? diary.selected?.id
      if (!id) return diary.say('no entry selected', 'error')
      const entry = diary.open?.id === id ? diary.open : diary.selected
      if (entry?.shared) return diary.say('already shared — :link to copy, :unshare to revoke')
      await diary.guard(() => diary.toggleShare(id))
    },
  },
  {
    name: 'unshare',
    group: 'sharing',
    help: 'revoke the share link',
    run: async () => {
      const id = diary.open?.id ?? diary.selected?.id
      if (!id) return diary.say('no entry selected', 'error')
      const entry = diary.open?.id === id ? diary.open : diary.selected
      if (!entry?.shared) return diary.say('entry is not shared')
      await diary.guard(() => diary.toggleShare(id))
    },
  },
  {
    name: 'link',
    group: 'sharing',
    help: 'copy the share link of the current entry',
    run: async () => {
      const token = diary.open?.share_token ?? diary.selected?.share_token
      if (!token) return diary.say('entry is not shared — :share first', 'error')
      const url = `${location.origin}/s/${token}`
      diary.say((await diary.copy(url)) ? `copied ${url}` : url)
    },
  },
  {
    name: 'copy',
    aliases: ['yank'],
    group: 'sharing',
    help: 'copy the whole entry to the clipboard as markdown',
    run: async () => {
      if (!requireOpen()) return
      const { title, body } = diary.draft
      const text = title.trim() ? `# ${title.trim()}\n\n${body}` : body
      diary.say((await diary.copy(text)) ? 'entry copied' : 'the clipboard said no', 'info')
    },
  },
  {
    name: 'export',
    aliases: ['exp'],
    group: 'sharing',
    help: 'download the open entry as a .md file',
    run: () => {
      if (!requireOpen()) return
      const { title, body, created_at } = diary.draft
      const text = title.trim() ? `# ${title.trim()}\n\n${body}` : body
      const url = URL.createObjectURL(new Blob([text], { type: 'text/markdown' }))
      const anchor = document.createElement('a')
      anchor.href = url
      anchor.download = exportName(title.trim(), created_at)
      anchor.click()
      URL.revokeObjectURL(url)
      diary.say(`wrote ${anchor.download}`)
    },
  },
  {
    name: 'upload',
    aliases: ['up'],
    group: 'editing',
    help: 'pick files to attach at the cursor',
    run: () => hooks.pickFiles(),
  },
  {
    name: 'media',
    group: 'editing',
    help: 'browse everything you have uploaded',
    run: () => diary.guard(async () => {
      await diary.loadMedia()
      diary.overlay = 'media'
    }),
  },
  {
    name: 'search',
    aliases: ['se'],
    args: '[text]',
    group: 'entries',
    help: 'full-text search (empty clears)',
    run: (arg) => diary.guard(() => diary.search(arg)),
  },
  {
    name: 'clear',
    aliases: ['noh'],
    group: 'entries',
    help: 'clear the search filter',
    run: () => diary.guard(() => diary.search('')),
  },
  {
    name: 'reload',
    aliases: ['r'],
    group: 'entries',
    help: 're-read the entry list from the server',
    run: () =>
      diary.guard(async () => {
        await diary.refresh()
        diary.say(`${diary.entries.length} entries`)
      }),
  },
  {
    name: 'stats',
    group: 'view',
    help: 'word, line and entry counts',
    run: () => {
      const body = diary.open ? diary.draft.body : ''
      const words = body.split(/\s+/).filter(Boolean).length
      const here = diary.open ? `entry:${diary.open.id} ${words}w ${body.split('\n').length}L · ` : ''
      diary.say(`${here}${diary.entries.length} entries${diary.query ? ` matching "${diary.query}"` : ''}`)
    },
  },
  {
    name: 'theme',
    args: '<mocha|green|amber|ice>',
    group: 'view',
    help: 'switch the colour scheme',
    run: (arg) => setTheme(arg.trim() || diary.theme),
  },
  {
    name: 'set',
    args: '<option>',
    group: 'view',
    help: 'theme=mocha|green|amber|ice, vim, novim',
    run: (arg) => {
      const option = arg.trim()
      if (option === 'vim') return diary.setVim(true), diary.say('vim keys on')
      if (option === 'novim') return diary.setVim(false), diary.say('vim keys off')
      const theme = /^theme=(\w+)$/.exec(option)?.[1]
      if (theme) return setTheme(theme)
      diary.say(`unknown option: ${option}`, 'error')
    },
  },
  {
    name: 'help',
    aliases: ['h'],
    group: 'view',
    help: 'show the key and command reference',
    run: () => {
      diary.overlay = diary.overlay === 'help' ? 'none' : 'help'
    },
  },
  {
    name: 'logout',
    group: 'view',
    help: 'end the session',
    run: () => diary.guard(() => diary.logout()),
  },
]

const lookup = new Map<string, CommandSpec>()
for (const spec of commands) {
  lookup.set(spec.name, spec)
  for (const alias of spec.aliases ?? []) lookup.set(alias, spec)
}

/** Run an ex-command line such as `w`, `date 2026-01-01` or `set theme=amber`. */
export async function runCommand(line: string): Promise<void> {
  const input = line.trim().replace(/^:/, '')
  if (!input) return

  if (input.startsWith('/')) {
    await diary.guard(() => diary.search(input.slice(1)))
    return
  }

  const [head, ...rest] = input.split(/\s+/)
  const arg = rest.join(' ')

  // `:12` jumps to an entry by id, the way `:12` jumps to a line in vim.
  if (/^\d+$/.test(head)) {
    await diary.guard(() => diary.openEntry(Number(head)))
    return
  }

  // `:q!`, `:quit!` — the bang is part of the command name, not an argument.
  const forced = head.endsWith('!')
  const spec = lookup.get(forced ? head.slice(0, -1) : head)
  if (!spec) {
    diary.say(`E492: not an editor command: ${head}`, 'error')
    return
  }
  if (forced) {
    if (!spec.bang) return diary.say(`E477: no ! allowed: ${head}`, 'error')
    await spec.bang.run(arg)
    return
  }
  await spec.run(arg)
}

/** Command names for the `:` completion menu. */
export function completions(prefix: string): CommandSpec[] {
  const head = prefix.trim().replace(/^:/, '').split(/\s+/)[0] ?? ''
  if (!head) return commands
  return commands.filter(
    (spec) => spec.name.startsWith(head) || spec.aliases?.some((a) => a.startsWith(head)),
  )
}
