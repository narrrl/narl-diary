import { diary, type Theme } from './store.svelte'
import { parseDay } from './util'

/** Filled in by App.svelte so ex-commands can reach the DOM-bound bits. */
export const hooks = {
  pickFiles: () => {},
  focusEditor: () => {},
  focusList: () => {},
  insertText: (_text: string) => {},
  openCommandLine: (_initial: string) => {},
}

export interface CommandSpec {
  name: string
  aliases?: string[]
  args?: string
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

export const commands: CommandSpec[] = [
  {
    name: 'write',
    aliases: ['w'],
    help: 'save the open entry',
    run: () => diary.guard(() => diary.save()),
  },
  {
    name: 'wq',
    aliases: ['x'],
    help: 'save and leave insert/editor mode',
    run: async () => {
      await diary.guard(() => diary.save())
      diary.editing = false
      hooks.focusList()
    },
  },
  {
    name: 'quit',
    aliases: ['q'],
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
    name: 'edit',
    aliases: ['e'],
    args: '[id]',
    help: 'edit the open entry, or open an entry by id',
    run: async (arg) => {
      const id = arg ? Number(arg) : diary.open?.id ?? diary.selected?.id
      if (!id || Number.isNaN(id)) return diary.say('usage: :e <id>', 'error')
      await diary.guard(() => diary.openEntry(id, true))
      hooks.focusEditor()
    },
  },
  {
    name: 'delete',
    aliases: ['d', 'rm'],
    args: '[id]',
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
    help: 'copy the share link of the current entry',
    run: async () => {
      const token = diary.open?.share_token ?? diary.selected?.share_token
      if (!token) return diary.say('entry is not shared — :share first', 'error')
      const url = `${location.origin}/s/${token}`
      diary.say((await diary.copy(url)) ? `copied ${url}` : url)
    },
  },
  {
    name: 'upload',
    aliases: ['up'],
    help: 'pick files to attach at the cursor',
    run: () => hooks.pickFiles(),
  },
  {
    name: 'media',
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
    help: 'full-text search (empty clears)',
    run: (arg) => diary.guard(() => diary.search(arg)),
  },
  {
    name: 'set',
    args: '<option>',
    help: 'theme=mocha|green|amber|ice, vim, novim',
    run: (arg) => {
      const option = arg.trim()
      if (option === 'vim') return diary.setVim(true), diary.say('vim keys on')
      if (option === 'novim') return diary.setVim(false), diary.say('vim keys off')
      const theme = /^theme=(mocha|green|amber|ice)$/.exec(option)?.[1]
      if (theme) return diary.setTheme(theme as Theme), diary.say(`theme ${theme}`)
      diary.say(`unknown option: ${option}`, 'error')
    },
  },
  {
    name: 'help',
    aliases: ['h'],
    help: 'show the key and command reference',
    run: () => {
      diary.overlay = diary.overlay === 'help' ? 'none' : 'help'
    },
  },
  {
    name: 'logout',
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
