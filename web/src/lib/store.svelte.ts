import {
  api,
  ApiError,
  type Board,
  type Card,
  type Crumb,
  type WorkspaceNode,
  type MediaFile,
  type NodeSummary,
} from './api'

export type Pane = 'list' | 'document'
/** The tree of documents, or the board of the space being browsed. */
export type View = 'tree' | 'board'
export type Flash = { text: string; kind: 'info' | 'error' } | null
export type Theme = 'mocha' | 'green' | 'amber' | 'ice'

/**
 * The whole application state. It is deliberately one object: the keyboard
 * layer needs to see everything at once to decide what a key means.
 */
class Workspace {
  user = $state<string | null>(null)
  booting = $state(true)

  /** The top level: diary, work, uni, whatever else gets made. */
  spaces = $state<NodeSummary[]>([])
  /** The space being browsed, and the folder inside it the list is showing. */
  spaceId = $state<number | null>(null)
  parentId = $state<number | null>(null)
  /** Ancestors of the current folder, the space first and the folder last. */
  path = $state<Crumb[]>([])

  /** Children of the current folder, or the results of a search. */
  nodes = $state<NodeSummary[]>([])
  cursor = $state(0)
  query = $state('')

  open = $state<WorkspaceNode | null>(null)
  draft = $state({ name: '', body: '', created_at: 0 })
  editing = $state(false)
  /** Set when the editor should open in insert mode rather than normal mode. */
  enterInsert = $state(false)
  dirty = $state(false)

  /** The board of the current space, loaded only while it is being looked at. */
  view = $state<View>('tree')
  board = $state<Board | null>(null)
  listCursor = $state(0)
  cardCursor = $state(0)

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

  get selected(): NodeSummary | undefined {
    return this.nodes[this.cursor]
  }

  get space(): NodeSummary | undefined {
    return this.spaces.find((s) => s.id === this.spaceId)
  }

  /** `work/einarbeiten` — what the sidebar and the status line show. */
  get here(): string {
    return this.path.map((crumb) => crumb.slug).join('/')
  }

  get list() {
    return this.board?.lists[this.listCursor]
  }

  get card(): Card | undefined {
    return this.list?.cards[this.cardCursor]
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
    this.#autosave = setTimeout(() => void this.#writeBack(), Workspace.AUTOSAVE_MS)
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
      if (!raw) return null
      const stashed = JSON.parse(raw)
      // Drafts stashed before documents had names carry a title instead.
      return { name: stashed.name ?? stashed.title ?? '', body: stashed.body, created_at: stashed.created_at }
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
   * Without this, clicking another document in the list discarded unsaved work
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
      await this.loadSpaces(Workspace.addressedPath())
    } catch {
      this.user = null
    } finally {
      this.booting = false
    }
  }

  async login(username: string, password: string) {
    const { user } = await api.login(username, password)
    this.user = user
    await this.loadSpaces(Workspace.addressedPath())
  }

  async logout() {
    await api.logout()
    this.user = null
    this.spaces = []
    this.nodes = []
    this.open = null
    this.editing = false
  }

  /*
   * Navigation. The list pane always shows one folder's children, so browsing
   * is: pick a space, descend into folders, come back up. A search steps
   * outside that and shows matches from anywhere in the current space.
   */

  /** The slug path in the address bar, if the page was opened on a `/n/...` URL. */
  static addressedPath(): string | null {
    return location.pathname.startsWith('/n/')
      ? decodeURIComponent(location.pathname.slice(3)).replace(/\/$/, '')
      : null
  }

  /** Keep the address bar on the node being looked at, so it can be linked to. */
  #syncUrl(path: Crumb[]) {
    const slugs = path.map((crumb) => crumb.slug).join('/')
    if (slugs) history.replaceState(null, '', `/n/${slugs}`)
  }

  /** Open whatever `work/einarbeiten/01-was-ist-eebus` names. */
  async openPath(path: string) {
    const node = await api.resolvePath(path)
    if (node.kind === 'document') {
      if (node.parent_id !== null) await this.goTo(node.parent_id)
      await this.openNode(node.id)
    } else {
      await this.goTo(node.id)
    }
  }

  async loadSpaces(startPath: string | null = null) {
    this.spaces = await api.listSpaces()
    if (startPath) {
      try {
        await this.openPath(startPath)
        return
      } catch {
        // A stale or hand-typed address should not leave the app on nothing.
        this.say(`no such path: ${startPath}`, 'error')
      }
    }
    const remembered = Number(localStorage.getItem('diary:space'))
    const start =
      this.spaces.find((s) => s.id === remembered) ??
      this.spaces.find((s) => s.slug === 'diary') ??
      this.spaces[0]
    if (start) await this.enterSpace(start.id)
  }

  async enterSpace(id: number) {
    this.spaceId = id
    localStorage.setItem('diary:space', String(id))
    this.query = ''
    await this.goTo(id)
    // The board belongs to the space, so switching space while looking at one
    // shows the new space's board — or drops back to the tree for a space that
    // has none, rather than leaving the previous board on screen.
    if (this.view === 'board') {
      this.listCursor = 0
      this.cardCursor = 0
      if (this.space?.has_board) await this.reloadBoard(id, undefined)
      else this.closeBoard()
    }
  }

  /** Show the children of `id`, which must be a space or a folder. */
  async goTo(id: number, keep?: number) {
    await this.flush()
    this.parentId = id
    this.query = ''
    const folder = await api.getNode(id)
    this.path = folder.path
    this.spaceId = folder.path[0]?.id ?? id
    this.nodes = await api.listNodes({ parent: id })
    const index = this.nodes.findIndex((n) => n.id === keep)
    this.cursor = index >= 0 ? index : 0
    this.#syncUrl(folder.path)
  }

  /** Up one level, selecting the folder just left so `h` then `l` is a no-op. */
  async up() {
    const from = this.path.at(-1)
    const parent = this.path.at(-2)
    if (!parent || !from) return
    await this.goTo(parent.id, from.id)
  }

  async refresh() {
    const keepId = this.selected?.id
    this.nodes = this.query
      ? await api.listNodes({ q: this.query, space: this.spaceId ?? undefined })
      : await api.listNodes({ parent: this.parentId ?? undefined })
    const index = this.nodes.findIndex((n) => n.id === keepId)
    this.cursor = index >= 0 ? index : Math.min(this.cursor, Math.max(this.nodes.length - 1, 0))
  }

  async search(query: string) {
    this.query = query
    this.cursor = 0
    await this.refresh()
  }

  move(delta: number) {
    if (this.nodes.length === 0) return
    this.cursor = Math.min(Math.max(this.cursor + delta, 0), this.nodes.length - 1)
  }

  async openNode(id: number, edit = false) {
    if (this.open?.id !== id) await this.flush()

    const node = await api.getNode(id)
    if (node.kind !== 'document') {
      await this.goTo(node.id)
      return
    }

    this.open = node
    this.draft = { name: node.name, body: node.body, created_at: node.created_at }
    this.dirty = false
    this.editing = edit
    this.pane = 'document'
    this.#syncUrl(node.path)
    const index = this.nodes.findIndex((n) => n.id === id)
    if (index >= 0) this.cursor = index

    // A stash that differs from what the server has is work the last session
    // did not manage to write. Vim would call this a swap file.
    const stashed = this.#readStash(id)
    if (stashed && (stashed.body !== node.body || stashed.name !== node.name)) {
      this.draft = stashed
      this.dirty = true
      this.say('recovered an unsaved draft — :w to keep it, :e! to discard', 'error')
    } else {
      this.dropStash(id)
    }
  }

  /** What Enter does: descend into a folder, open a document. */
  async openSelected(edit = false) {
    const selected = this.selected
    if (!selected) return
    if (selected.kind === 'document') await this.openNode(selected.id, edit)
    else await this.goTo(selected.id)
  }

  /** A new document is created immediately so uploads have something to attach to. */
  async createDocument(name = '') {
    const parent = this.parentId
    if (parent === null) return
    const node = await api.createNode({ parent_id: parent, name, body: '' })
    this.query = ''
    await this.refresh()
    await this.openNode(node.id, true)
    this.enterInsert = true
    this.say(`new document ${this.here}/${node.slug}`)
  }

  async createFolder(name: string) {
    const parent = this.parentId
    if (parent === null) return
    const node = await api.createNode({ parent_id: parent, kind: 'folder', name })
    await this.refresh()
    const index = this.nodes.findIndex((n) => n.id === node.id)
    if (index >= 0) this.cursor = index
    this.say(`new folder ${this.here}/${node.slug}`)
  }

  async createSpace(name: string) {
    const space = await api.createSpace(name)
    await this.loadSpaces()
    await this.enterSpace(space.id)
    this.say(`new space ${space.slug}`)
  }

  async save({ quiet = false } = {}) {
    if (!this.open) return
    // Snapshot what goes over the wire: an autosave races the typing that
    // triggered it, and anything typed since must stay marked unsaved.
    const sent = { ...this.draft, name: this.draft.name.trim() }
    const saved = await api.updateNode(this.open.id, sent)

    this.open = saved
    this.dirty =
      this.draft.body !== sent.body ||
      this.draft.name.trim() !== sent.name ||
      this.draft.created_at !== sent.created_at
    if (this.dirty) this.touch()
    else this.dropStash(saved.id)
    // The response carries the same excerpt the list query builds, so the row
    // can usually be patched in place rather than re-reading the whole list.
    if (!this.patch(saved)) await this.refresh()
    if (!quiet) this.say(`"${saved.slug}" ${saved.body.split('\n').length}L written`)
  }

  /**
   * Fold a freshly saved document back into its list row. Returns false when
   * the row cannot be patched in place — a new document, or one whose date or
   * slug moved it somewhere else — and the caller should re-read the list.
   */
  patch(node: WorkspaceNode): boolean {
    const index = this.nodes.findIndex((n) => n.id === node.id)
    if (index < 0) return false
    const row = this.nodes[index]
    if (row.created_at !== node.created_at || row.slug !== node.slug) return false
    const { body: _body, path: _path, ...summary } = node
    this.nodes[index] = summary
    return true
  }

  async deleteNode(id: number) {
    if (this.#autosave) {
      clearTimeout(this.#autosave)
      this.#autosave = null
    }
    this.dropStash(id)
    const { removed } = await api.deleteNode(id)
    if (this.open?.id === id) {
      this.open = null
      this.editing = false
      this.pane = 'list'
    }
    await this.refresh()
    this.say(removed > 1 ? `deleted ${removed} nodes` : `deleted #${id}`)
  }

  /** Reparent the node under the cursor, for `:mv`. */
  async moveNode(id: number, parentId: number) {
    await api.moveNode(id, parentId)
    await this.refresh()
    this.say(`moved #${id}`)
  }

  /*
   * Boards. One per space, which is what makes "what am I working on" a
   * question with an answer: work and uni are asked separately, and the diary
   * is not asked at all. The board is reloaded whole after every change — it
   * is a few dozen rows, and a board that disagrees with the server about
   * where a card is would be worse than a round trip.
   */

  async openBoard() {
    const space = this.space
    if (!space) return
    if (!space.has_board) {
      return this.say(`${space.slug} has no board — :set board turns one on`, 'error')
    }
    await this.reloadBoard(space.id)
    this.view = 'board'
  }

  closeBoard() {
    this.view = 'tree'
  }

  async toggleBoardView() {
    if (this.view === 'board') this.closeBoard()
    else await this.openBoard()
  }

  async reloadBoard(spaceId = this.spaceId, keep = this.card?.id) {
    if (spaceId === null) return
    this.board = await api.board(spaceId)
    // Follow the card that was just moved rather than whatever now sits under
    // the cursor, so J J J walks one card down instead of shuffling three.
    if (keep !== undefined) {
      const at = this.locate(keep)
      if (at) {
        this.listCursor = at.list
        this.cardCursor = at.card
      }
    }
    this.clampBoard()
  }

  /** Where a card sits now, as list and card indices. */
  locate(cardId: number): { list: number; card: number } | null {
    const lists = this.board?.lists ?? []
    for (let list = 0; list < lists.length; list++) {
      const card = lists[list].cards.findIndex((c) => c.id === cardId)
      if (card >= 0) return { list, card }
    }
    return null
  }

  clampBoard() {
    const lists = this.board?.lists ?? []
    this.listCursor = Math.min(Math.max(this.listCursor, 0), Math.max(lists.length - 1, 0))
    const cards = lists[this.listCursor]?.cards ?? []
    this.cardCursor = Math.min(Math.max(this.cardCursor, 0), Math.max(cards.length - 1, 0))
  }

  /** `h`/`l`: to the list next door, keeping the cursor as near as it can. */
  moveList(delta: number) {
    const lists = this.board?.lists ?? []
    if (lists.length === 0) return
    this.listCursor = Math.min(Math.max(this.listCursor + delta, 0), lists.length - 1)
    this.clampBoard()
  }

  /** `j`/`k`: down and up the cards of the current list. */
  moveCard(delta: number) {
    const cards = this.list?.cards ?? []
    if (cards.length === 0) return
    this.cardCursor = Math.min(Math.max(this.cardCursor + delta, 0), cards.length - 1)
  }

  /** `J`/`K`: take the card with you instead of leaving it behind. */
  async nudgeCard(delta: number) {
    const card = this.card
    const list = this.list
    if (!card || !list) return
    const to = this.cardCursor + delta
    if (to < 0 || to >= list.cards.length) return
    await api.moveCard(card.id, list.id, to)
    await this.reloadBoard(this.spaceId, card.id)
  }

  /** `H`/`L`: the same card, one list over — the move a board is really for. */
  async sendCard(delta: number) {
    const card = this.card
    const lists = this.board?.lists ?? []
    const target = lists[this.listCursor + delta]
    if (!card || !target) return
    await api.moveCard(card.id, target.id, target.cards.length)
    await this.reloadBoard(this.spaceId, card.id)
    this.say(`"${card.title}" → ${target.name}`)
  }

  async createCard(title: string, nodeId: number | null = null) {
    const list = this.list
    if (!list) return this.say('no list to put it on', 'error')
    const card = await api.createCard(list.id, { title, node_id: nodeId })
    await this.reloadBoard(this.spaceId, card.id)
    this.say(nodeId === null ? `carded "${card.title}"` : `carded "${card.title}" → #${nodeId}`)
  }

  async patchCard(patch: Parameters<typeof api.updateCard>[1]) {
    const card = this.card
    if (!card) return this.say('no card selected', 'error')
    await api.updateCard(card.id, patch)
    await this.reloadBoard(this.spaceId, card.id)
  }

  async toggleDone() {
    const card = this.card
    if (!card) return this.say('no card selected', 'error')
    await api.updateCard(card.id, { done: card.done_at === null })
    await this.reloadBoard(this.spaceId, card.id)
    this.say(card.done_at === null ? `done: ${card.title}` : `reopened: ${card.title}`)
  }

  async deleteCard() {
    const card = this.card
    if (!card) return this.say('no card selected', 'error')
    await api.deleteCard(card.id)
    await this.reloadBoard(this.spaceId, undefined)
    this.say(`deleted "${card.title}"`)
  }

  async createList(name: string) {
    if (this.spaceId === null) return
    this.board = await api.createList(this.spaceId, name)
    this.listCursor = this.board.lists.length - 1
    this.cardCursor = 0
    this.say(`new list ${name}`)
  }

  async deleteList() {
    const list = this.list
    if (!list) return this.say('no list selected', 'error')
    this.board = await api.deleteList(list.id)
    this.clampBoard()
    this.say(`deleted list ${list.name}`)
  }

  /** Open the document a card points at, leaving the board behind. */
  async openCard() {
    const card = this.card
    if (!card) return
    if (card.node_id === null) {
      return this.say(`"${card.title}" is not linked to a document — :card! links one`, 'error')
    }
    this.view = 'tree'
    await this.openNode(card.node_id)
  }

  /** Turn a space's board on or off. The diary ships with it off. */
  async setBoard(on: boolean) {
    const space = this.space
    if (!space) return
    await api.updateNode(space.id, { name: space.name, body: '', has_board: on })
    this.spaces = await api.listSpaces()
    if (on) await this.openBoard()
    else {
      this.closeBoard()
      this.board = null
    }
    this.say(on ? `${space.slug} has a board` : `${space.slug} has no board any more`)
  }

  async toggleShare(id: number) {
    const node = this.open?.id === id ? this.open : await api.getNode(id)
    if (node.shared) {
      await api.unshare(id)
      this.say(`#${id} is private again`)
    } else {
      const url = await this.mintShareLink(id)
      this.say(`shared → ${url} (copied)`)
    }
    if (this.open?.id === id) this.open = await api.getNode(id)
    await this.refresh()
  }

  /*
   * The server keeps only a hash of the key that lives in the link's fragment,
   * so a link exists in full exactly once: here, in the moment it is minted.
   * There is nothing to re-copy later — asking again mints a new link and
   * retires the old one.
   */
  async mintShareLink(id: number) {
    const { path } = await api.share(id)
    const url = `${location.origin}${path}`
    await this.copy(url)
    return url
  }

  async copy(text: string) {
    try {
      await navigator.clipboard.writeText(text)
      return true
    } catch {
      return false
    }
  }

  /**
   * Import a picked folder (or a zip of one) into the folder being browsed.
   * The report is worth a line of its own: an import that quietly skipped half
   * its files would only be noticed much later.
   */
  async importFolder(files: File[]) {
    const parent = this.parentId
    if (parent === null) return
    this.say(`importing ${files.length} file${files.length === 1 ? '' : 's'} …`)

    const report = await api.importFolder(parent, files)
    await this.refresh()

    const parts = [
      `${report.documents} document${report.documents === 1 ? '' : 's'}`,
      `${report.folders} folder${report.folders === 1 ? '' : 's'}`,
      `${report.files} file${report.files === 1 ? '' : 's'}`,
      `${report.links} link${report.links === 1 ? '' : 's'} rewritten`,
    ]
    if (report.skipped.length > 0) parts.push(`${report.skipped.length} skipped`)
    this.say(`imported ${parts.join(', ')}`)
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

export const workspace = new Workspace()
