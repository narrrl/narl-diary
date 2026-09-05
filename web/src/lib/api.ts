export interface EntrySummary {
  id: number
  title: string
  excerpt: string
  created_at: number
  updated_at: number
  shared: boolean
  share_token: string | null
}

export interface Entry {
  id: number
  title: string
  body: string
  excerpt: string
  created_at: number
  updated_at: number
  shared: boolean
  share_token: string | null
}

export interface SharedEntry {
  title: string
  body: string
  created_at: number
  updated_at: number
  token: string
}

export interface MediaFile {
  id: string
  filename: string
  mime: string
  size: number
  created_at: number
  url: string
  /** Every entry that embeds this file — a file may be used by several. */
  entry_ids: number[]
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

  listEntries: (q = '') =>
    request<EntrySummary[]>(`/entries${q ? `?q=${encodeURIComponent(q)}` : ''}`),

  getEntry: (id: number) => request<Entry>(`/entries/${id}`),

  createEntry: (input: { title: string; body: string; created_at?: number }) =>
    request<Entry>('/entries', { method: 'POST', body: body(input) }),

  updateEntry: (id: number, input: { title: string; body: string; created_at?: number }) =>
    request<Entry>(`/entries/${id}`, { method: 'PUT', body: body(input) }),

  deleteEntry: (id: number) => request<{ ok: true }>(`/entries/${id}`, { method: 'DELETE' }),

  share: (id: number) => request<{ token: string; path: string }>(`/entries/${id}/share`, { method: 'POST' }),

  unshare: (id: number) => request<{ ok: true }>(`/entries/${id}/share`, { method: 'DELETE' }),

  listMedia: () => request<MediaFile[]>('/media'),

  upload: (files: File[]) => {
    const form = new FormData()
    for (const file of files) form.append('file', file)
    return request<MediaFile[]>('/media', { method: 'POST', body: form })
  },

  deleteMedia: (id: string) => request<{ ok: true }>(`/media/${id}`, { method: 'DELETE' }),

  backupStatus: () => request<BackupStatus>('/backup'),

  backupNow: () => request<BackupStatus>('/backup', { method: 'POST' }),

  readShared: (token: string) => request<SharedEntry>(`/share/${encodeURIComponent(token)}`),
}
