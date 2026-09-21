/** What a node is: a top-level space, a folder inside one, or a document. */
export type NodeKind = 'space' | 'folder' | 'document'

export interface NodeSummary {
  id: number
  parent_id: number | null
  kind: NodeKind
  name: string
  slug: string
  excerpt: string
  position: number
  has_board: boolean
  child_count: number
  created_at: number
  updated_at: number
  shared: boolean
  share_token: string | null
}

/** One step of a node's path, the space first and the node itself last. */
export interface Crumb {
  id: number
  kind: NodeKind
  name: string
  slug: string
}

export interface WorkspaceNode extends NodeSummary {
  body: string
  path: Crumb[]
}

/** A public key allowed to mount the workspace over SFTP. */
export interface SshKey {
  id: number
  name: string
  fingerprint: string
  created_at: number
  last_used_at: number | null
}

export interface SharedDocument {
  name: string
  body: string
  created_at: number
  updated_at: number
  token: string
}

/** What an import did, as `POST /api/nodes/{id}/import` reports it. */
export interface ImportReport {
  folders: number
  documents: number
  files: number
  links: number
  skipped: string[]
}

/** A card on a space's board. `node_id` is the document it is about, if any. */
export interface Card {
  id: number
  list_id: number
  title: string
  body: string
  node_id: number | null
  node_name: string | null
  due_at: number | null
  done_at: number | null
  position: number
  created_at: number
  updated_at: number
}

export interface BoardList {
  id: number
  name: string
  position: number
  cards: Card[]
}

/** One space's board. There is one per space; the diary has none. */
export interface Board {
  space_id: number
  space_name: string
  lists: BoardList[]
}

export interface MediaFile {
  id: string
  filename: string
  mime: string
  size: number
  created_at: number
  url: string
  /** Every document that embeds this file — a file may be used by several. */
  node_ids: number[]
}

/** The Proton Drive mirror, as `/api/backup` reports it. */
export interface BackupStatus {
  configured: boolean
  device: string | null
  running: boolean
  pending: boolean
  last_run_at: number | null
  last_success_at: number | null
  last_error: string | null
  last: { uploaded: number; skipped: number; pruned: number; bytes: number } | null
}

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message)
  }
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(`/api${path}`, {
    credentials: 'same-origin',
    ...init,
    headers:
      init.body instanceof FormData
        ? init.headers
        : { 'content-type': 'application/json', ...init.headers },
  })

  if (!response.ok) {
    let message = response.statusText
    try {
      message = (await response.json()).error ?? message
    } catch {
      /* the body was not JSON; the status text will do */
    }
    throw new ApiError(response.status, message)
  }

  return response.status === 204 ? (undefined as T) : await response.json()
}

const body = (data: unknown) => JSON.stringify(data)

export const api = {
  me: () => request<{ user: string }>('/me'),

  login: (username: string, password: string) =>
    request<{ user: string }>('/login', { method: 'POST', body: body({ username, password }) }),

  logout: () => request<{ ok: true }>('/logout', { method: 'POST' }),

  listSpaces: () => request<NodeSummary[]>('/spaces'),

  createSpace: (name: string, has_board = true) =>
    request<WorkspaceNode>('/spaces', { method: 'POST', body: body({ name, has_board }) }),

  /** The children of a node, or — with `q` — a search across the whole tree. */
  listNodes: (params: { parent?: number; q?: string; space?: number }) => {
    const query = new URLSearchParams()
    if (params.q) query.set('q', params.q)
    else if (params.parent !== undefined) query.set('parent', String(params.parent))
    if (params.q && params.space !== undefined) query.set('space', String(params.space))
    return request<NodeSummary[]>(`/nodes?${query}`)
  },

  getNode: (id: number) => request<WorkspaceNode>(`/nodes/${id}`),

  /** `work/einarbeiten/01-was-ist-eebus` — the node behind a `/n/...` address. */
  resolvePath: (path: string) => request<WorkspaceNode>(`/resolve?path=${encodeURIComponent(path)}`),

  /**
   * Upload a folder into a node. The relative path is passed as each part's
   * file name, which is the only place a multipart body has to put it, and is
   * what the server rebuilds the directory tree from.
   */
  importFolder: (id: number, files: File[]) => {
    const form = new FormData()
    for (const file of files) form.append('file', file, file.webkitRelativePath || file.name)
    return request<ImportReport>(`/nodes/${id}/import`, { method: 'POST', body: form })
  },

  createNode: (input: {
    parent_id: number
    kind?: NodeKind
    name?: string
    body?: string
    created_at?: number
  }) => request<WorkspaceNode>('/nodes', { method: 'POST', body: body(input) }),

  updateNode: (
    id: number,
    input: { name: string; body: string; created_at?: number; has_board?: boolean },
  ) => request<WorkspaceNode>(`/nodes/${id}`, { method: 'PUT', body: body(input) }),

  deleteNode: (id: number) =>
    request<{ ok: true; removed: number }>(`/nodes/${id}`, { method: 'DELETE' }),

  moveNode: (id: number, parent_id: number, position?: number) =>
    request<WorkspaceNode>(`/nodes/${id}/move`, { method: 'POST', body: body({ parent_id, position }) }),

  /** Publishes the document, or mints a fresh key for one already published. */
  share: (id: number) =>
    request<{ token: string; key: string; path: string }>(`/nodes/${id}/share`, { method: 'POST' }),

  unshare: (id: number) => request<{ ok: true }>(`/nodes/${id}/share`, { method: 'DELETE' }),

  /** The board of a space. Fails for a space that has none, such as the diary. */
  board: (spaceId: number) => request<Board>(`/spaces/${spaceId}/board`),

  createList: (spaceId: number, name: string) =>
    request<Board>(`/spaces/${spaceId}/lists`, { method: 'POST', body: body({ name }) }),

  renameList: (listId: number, name: string) =>
    request<Board>(`/lists/${listId}`, { method: 'PUT', body: body({ name }) }),

  deleteList: (listId: number) => request<Board>(`/lists/${listId}`, { method: 'DELETE' }),

  createCard: (
    listId: number,
    input: { title: string; body?: string; node_id?: number | null; due_at?: number | null },
  ) => request<Card>(`/lists/${listId}/cards`, { method: 'POST', body: body(input) }),

  /*
   * Only what changed is sent. `due_at: null` clears the date while leaving it
   * out keeps it, which is why the server reads those fields as double options.
   */
  updateCard: (
    id: number,
    patch: {
      title?: string
      body?: string
      node_id?: number | null
      due_at?: number | null
      done?: boolean
    },
  ) => request<Card>(`/cards/${id}`, { method: 'PUT', body: body(patch) }),

  deleteCard: (id: number) => request<{ ok: true }>(`/cards/${id}`, { method: 'DELETE' }),

  moveCard: (id: number, list_id: number, position?: number) =>
    request<Card>(`/cards/${id}/move`, { method: 'POST', body: body({ list_id, position }) }),

  listMedia: () => request<MediaFile[]>('/media'),

  upload: (files: File[]) => {
    const form = new FormData()
    for (const file of files) form.append('file', file)
    return request<MediaFile[]>('/media', { method: 'POST', body: form })
  },

  deleteMedia: (id: string) => request<{ ok: true }>(`/media/${id}`, { method: 'DELETE' }),

  backupStatus: () => request<BackupStatus>('/backup'),

  backupNow: () => request<BackupStatus>('/backup', { method: 'POST' }),

  readShared: (token: string, key: string) =>
    request<SharedDocument>(`/share/${encodeURIComponent(token)}/${encodeURIComponent(key)}`),

  listSshKeys: () => request<SshKey[]>('/ssh-keys'),

  addSshKey: (name: string, public_key: string) =>
    request<SshKey>('/ssh-keys', { method: 'POST', body: body({ name, public_key }) }),

  removeSshKey: (id: number) => request<{ ok: true }>(`/ssh-keys/${id}`, { method: 'DELETE' }),
}
