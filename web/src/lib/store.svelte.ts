import { api, ApiError, type Entry, type EntrySummary, type MediaFile } from './api'

export type Pane = 'list' | 'entry'
export type Flash = { text: string; kind: 'info' | 'error' } | null
export type Theme = 'mocha' | 'green' | 'amber' | 'ice'

/**
 * The whole application state. It is deliberately one object: the keyboard
 * layer needs to see everything at once to decide what a key means.
 */
class Diary {
  user = $state<string | null>(null)
  booting = $state(true)

  entries = $state<EntrySummary[]>([])
  cursor = $state(0)
  query = $state('')

  open = $state<Entry | null>(null)
  draft = $state({ title: '', body: '', created_at: 0 })
  editing = $state(false)
  /** Set when the editor should open in insert mode rather than normal mode. */
  enterInsert = $state(false)
  dirty = $state(false)

  pane = $state<Pane>('list')
  vimMode = $state('normal')
  flash = $state<Flash>(null)

  overlay = $state<'none' | 'help' | 'media'>('none')
  media = $state<MediaFile[]>([])

  theme = $state<Theme>((localStorage.getItem('diary:theme') as Theme) ?? 'mocha')

  vimEnabled = $state(localStorage.getItem('diary:vim') !== 'off')

  /** Mirror vim yanks and deletes into the system clipboard, like `set clipboard=unnamed`. */
  clipboard = $state(localStorage.getItem('diary:clipboard') !== 'off')

  get selected(): EntrySummary | undefined {
    return this.entries[this.cursor]
  }

  say(text: string, kind: 'info' | 'error' = 'info') {
    this.flash = { text, kind }
    if (kind === 'info') {
      const current = this.flash
      setTimeout(() => {
        if (this.flash === current) this.flash = null
      }, 4000)
    }
  }

  async boot() {
    try {
      const { user } = await api.me()
      this.user = user
      await this.refresh()
    } catch {
      this.user = null
    } finally {
      this.booting = false
    }
  }

  async login(username: string, password: string) {
    const { user } = await api.login(username, password)
    this.user = user
    await this.refresh()
  }

  async logout() {
    await api.logout()
    this.user = null
    this.entries = []
    this.open = null
    this.editing = false
  }

  async refresh() {
    const keepId = this.selected?.id
    this.entries = await api.listEntries(this.query)
    const index = this.entries.findIndex((e) => e.id === keepId)
    this.cursor = index >= 0 ? index : Math.min(this.cursor, Math.max(this.entries.length - 1, 0))
  }

  async search(query: string) {
    this.query = query
    this.cursor = 0
    await this.refresh()
  }

  move(delta: number) {
    if (this.entries.length === 0) return
    this.cursor = Math.min(Math.max(this.cursor + delta, 0), this.entries.length - 1)
  }

  async openEntry(id: number, edit = false) {
    const entry = await api.getEntry(id)
    this.open = entry
    this.draft = { title: entry.title, body: entry.body, created_at: entry.created_at }
    this.dirty = false
    this.editing = edit
    this.pane = 'entry'
    const index = this.entries.findIndex((e) => e.id === id)
    if (index >= 0) this.cursor = index
  }

  async openSelected(edit = false) {
    const selected = this.selected
    if (selected) await this.openEntry(selected.id, edit)
  }

  /** A new entry is created immediately so uploads have something to attach to. */
  async createEntry() {
    const entry = await api.createEntry({ title: '', body: '' })
    this.query = ''
    await this.refresh()
    await this.openEntry(entry.id, true)
    this.enterInsert = true
    this.say(`new entry #${entry.id}`)
  }

  async save() {
    if (!this.open) return
    const saved = await api.updateEntry(this.open.id, {
      title: this.draft.title.trim(),
      body: this.draft.body,
      created_at: this.draft.created_at,
    })
    this.open = saved
    this.dirty = false
    await this.refresh()
    this.say(`"entry:${saved.id}" ${saved.body.split('\n').length}L written`)
  }

  async deleteEntry(id: number) {
    await api.deleteEntry(id)
    if (this.open?.id === id) {
      this.open = null
      this.editing = false
      this.pane = 'list'
    }
    await this.refresh()
    this.say(`entry #${id} deleted`)
  }

  async toggleShare(id: number) {
    const entry = this.open?.id === id ? this.open : await api.getEntry(id)
    if (entry.shared) {
      await api.unshare(id)
      this.say(`entry #${id} is private again`)
    } else {
      const { token } = await api.share(id)
      const url = `${location.origin}/s/${token}`
      await this.copy(url)
      this.say(`shared → ${url} (copied)`)
    }
    if (this.open?.id === id) this.open = await api.getEntry(id)
    await this.refresh()
  }

  async copy(text: string) {
    try {
      await navigator.clipboard.writeText(text)
      return true
    } catch {
      return false
    }
  }

  async upload(files: File[]): Promise<MediaFile[]> {
    const uploaded = await api.upload(files)
    this.say(`uploaded ${uploaded.length} file${uploaded.length === 1 ? '' : 's'}`)
    return uploaded
  }

  async loadMedia() {
    this.media = await api.listMedia()
  }

  setTheme(theme: Theme) {
    this.theme = theme
    localStorage.setItem('diary:theme', theme)
  }

  setVim(enabled: boolean) {
    this.vimEnabled = enabled
    localStorage.setItem('diary:vim', enabled ? 'on' : 'off')
  }

  setClipboard(enabled: boolean) {
    this.clipboard = enabled
    localStorage.setItem('diary:clipboard', enabled ? 'on' : 'off')
  }

  /** Surface a failed API call in the status line instead of the console. */
  async guard(action: () => Promise<unknown>) {
    try {
      await action()
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) {
        this.user = null
        this.say('session expired — log in again', 'error')
      } else {
        this.say(error instanceof Error ? error.message : String(error), 'error')
      }
    }
  }
}

export const diary = new Diary()
