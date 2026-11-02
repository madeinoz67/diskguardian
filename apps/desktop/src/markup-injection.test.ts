import { describe, expect, test } from 'bun:test'

// main.ts touches the document at import, so no bun test can mount it: the renderer's
// DOM assembly has no seam to drive. The guard is therefore source-level, and that is
// a stated limitation, not a virtue — what it pins is the class, not one call site.
// A disk-derived name went into HTML markup twice in this file's history: the legend
// heading (folded when the star was added) and then the hover label beside it, which
// survived that fold and was found by review. Until main.ts grows a DOM seam, "the
// member access appears nowhere" is the honest automated check available. Prose may
// name the class — the heading's comment does — code may not use the mechanism.
describe('main.ts never assigns markup', () => {
  test('no `.innerHTML` member access anywhere in the file', async () => {
    const source = await Bun.file(new URL('./main.ts', import.meta.url)).text()
    expect(source.includes('.innerHTML')).toBe(false)
  })
})
