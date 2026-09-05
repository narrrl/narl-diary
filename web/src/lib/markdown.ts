import DOMPurify from 'dompurify'
import { marked } from 'marked'

marked.setOptions({ gfm: true, breaks: true })

/**
 * Render entry markdown to sanitised HTML. When a share token is given, media
 * URLs are rewritten to the public route so the page works for a reader who has
 * no session.
 */
export function renderMarkdown(source: string, shareToken?: string): string {
  const md = shareToken
    ? source.replaceAll('/api/media/', `/api/share/${encodeURIComponent(shareToken)}/media/`)
    : source

  const html = marked.parse(md, { async: false }) as string

  return DOMPurify.sanitize(html, {
    ADD_TAGS: ['video', 'audio', 'source'],
    ADD_ATTR: ['controls', 'target', 'loading', 'playsinline', 'preload'],
  })
}

/** The markdown snippet that embeds a freshly uploaded file. */
export function embedSnippet(file: { url: string; filename: string; mime: string }): string {
  const name = file.filename.replaceAll(']', '')
  if (file.mime.startsWith('image/')) return `![${name}](${file.url})`
  if (file.mime.startsWith('video/')) return `<video src="${file.url}" controls playsinline></video>`
  if (file.mime.startsWith('audio/')) return `<audio src="${file.url}" controls></audio>`
  return `[${name}](${file.url})`
}
