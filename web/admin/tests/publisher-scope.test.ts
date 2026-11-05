import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { publisherAllowsPlugin } from '../src/api/publisher-scope'

describe('publisher release scope', () => {
  it('accepts only scoped names and exact official legacy identifiers', () => {
    for (const id of ['audio.play.process', 'hotkeys', 'l4d2.interactive', 'minecraft.server', 'peak.game', 'repo.game', 'sonicboom.server', 'textintel']) {
      expect(publisherAllowsPlugin('tiktools', id)).toBe(true)
      expect(publisherAllowsPlugin('other', id)).toBe(false)
    }
    expect(publisherAllowsPlugin('tiktools', 'tiktools.example')).toBe(true)
    expect(publisherAllowsPlugin('tiktools', 'tiktools.')).toBe(false)
    expect(publisherAllowsPlugin('tiktools', 'sonicboom.other')).toBe(false)
  })

  it('covers the verifier legacy allowlist so changes cannot silently drift', () => {
    const verifier = readFileSync(resolve(process.cwd(), '../../crates/cloud-verifier/src/lib.rs'), 'utf8')
    const list = verifier.match(/pub const OFFICIAL_LEGACY_IDS:.*?=\s*&\[([\s\S]*?)\];/)?.[1]
    expect(list).toBeDefined()
    const ids = [...list!.matchAll(/"([^"]+)"/g)].map((match) => match[1]!)
    expect(ids.length).toBeGreaterThan(0)
    for (const id of ids) expect(publisherAllowsPlugin('tiktools', id)).toBe(true)
  })
})
