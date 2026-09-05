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

  /** Pending debounced write, if the draft has changed since the last save. */
  #autosave: ReturnType<typeof setTimeout> | null = null

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

  /*
   * Autosave. Typing is where work gets lost, so a draft goes to localStorage
   * on every change — cheap, synchronous, survives a killed tab — and to the
   * server a couple of seconds after typing stops.
   */

  static readonly AUTOSAVE_MS = 2500

  #stashKey(id: number) {
    return `diary:draft:${id}`
  }

  /** Call whenever the draft changes. Replaces a bare `dirty = true`. */
  touch() {
    this.dirty = true
    this.stash()
    if (this.#autosave) clearTimeout(this.#autosave)
    this.#autosave = setTimeout(() => void this.#writeBack(), Diary.AUTOSAVE_MS)
  }

  stash() {
    if (!this.open) return
    try {
      localStorage.setItem(this.#stashKey(this.open.id), JSON.stringify(this.draft))
    } catch {
      /* a full or disabled localStorage must not stop the typing */
    }
  }

  dropStash(id: number) {
    try {
      localStorage.removeItem(this.#stashKey(id))
    } catch {
      /* nothing to do */
    }
  }

  #readStash(id: number): typeof this.draft | null {
    try {
      const raw = localStorage.getItem(this.#stashKey(id))
      return raw ? JSON.parse(raw) : null
    } catch {
      return null
    }
  }

  async #writeBack() {
    this.#autosave = null
    if (!this.open || !this.dirty) return
    await this.guard(() => this.save({ quiet: true }))
  }

  /**
   * Write out a pending draft before doing something that would replace it.
   * Without this, clicking another entry in the list discarded unsaved work
   * without a word.
   */
  async flush() {
    if (this.#autosave) {
      clearTimeout(this.#autosave)
      this.#autosave = null
    }
    if (this.open && this.dirty) await this.guard(() => this.save({ quiet: true }))
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
    if (this.open?.id !== id) await this.flush()

    const entry = await api.getEntry(id)
    this.open = entry
    this.draft = { title: entry.title, body: entry.body, created_at: entry.created_at }
    this.dirty = false
    this.editing = edit
    this.pane = 'entry'
    const index = this.entries.findIndex((e) => e.id === id)
    if (index >= 0) this.cursor = index

    // A stash that differs from what the server has is work the last session
    // did not manage to write. Vim would call this a swap file.
    const stashed = this.#readStash(id)
    if (stashed && (stashed.body !== entry.body || stashed.title !== entry.title)) {
      this.draft = stashed
      this.dirty = true
      this.say('recovered an unsaved draft — :w to keep it, :e! to discard', 'error')
    } else {
      this.dropStash(id)
    }
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

  async save({ quiet = false } = {}) {
    if (!this.open) return
    // Snapshot what goes over the wire: an autosave races the typing that
    // triggered it, and anything typed since must stay marked unsaved.
    const sent = { ...this.draft, title: this.draft.title.trim() }
    const saved = await api.updateEntry(this.open.id, sent)

    this.open = saved
    this.dirty =
      this.draft.body !== sent.body ||
      this.draft.title.trim() !== sent.title ||
      this.draft.created_at !== sent.created_at
    if (this.dirty) this.touch()
    else this.dropStash(saved.id)
    // The response carries the same excerpt the list query builds, so the row
    // can usually be patched in place rather than re-reading the whole list.
    if (!this.patch(saved)) await this.refresh()
    if (!quiet) this.say(`"entry:${saved.id}" ${saved.body.split('\n').length}L written`)
  }

  /**
   * Fold a freshly saved entry back into its list row. Returns false when the
   * row cannot be patched in place — a new entry, or one whose date moved it
   * somewhere else in the order — and the caller should re-read the list.
   */
  patch(entry: Entry): boolean {
    const index = this.entries.findIndex((e) => e.id === entry.id)
    if (index < 0 || this.entries[index].created_at !== entry.created_at) return false
    const { id, title, excerpt, created_at, updated_at, shared, share_token } = entry
    this.entries[index] = { id, title, excerpt, created_at, updated_at, shared, share_token }
    return true
  }

  async deleteEntry(id: number) {
    if (this.#autosave) {
      clearTimeout(this.#autosave)
      this.#autosave = null
    }
    this.dropStash(id)
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
