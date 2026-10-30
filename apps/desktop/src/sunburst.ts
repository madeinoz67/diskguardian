// The flower. One canvas, redrawn on demand: the arcs are cheap to draw and expensive
// to keep coherent, so state lives here and pixels are regenerated from it.

import { assignHues, colorFor, computeGeometry, hitTest, radiusOf } from './layout'
import type { Arc, Geometry } from './layout'
import type { ViewNode } from './types'
import { formatBytes } from './format'

export class Sunburst {
  private readonly canvas: HTMLCanvasElement
  private readonly context: CanvasRenderingContext2D
  private view: ViewNode[] = []
  private geometry: Geometry | null = null
  private hues: number[] = []
  private hovered: Arc | null = null
  private collected = new Set<number>()

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas
    const context = canvas.getContext('2d')
    if (!context) throw new Error('canvas 2d context unavailable')
    this.context = context
    this.resize()
  }

  setView(view: ViewNode[]): void {
    this.view = view
    this.hues = assignHues(view)
    this.hovered = null
    this.relayout()
    this.draw()
  }

  setCollected(ids: Iterable<number>): void {
    this.collected = new Set(ids)
    this.draw()
  }

  setHovered(arc: Arc | null): void {
    if (this.hovered?.node === arc?.node && this.hovered?.depth === arc?.depth) return
    this.hovered = arc
    this.draw()
  }

  getHovered(): Arc | null {
    return this.hovered
  }

  nodeAt(arc: Arc | null): ViewNode | null {
    if (!arc) return null
    return this.view[arc.node] ?? null
  }

  arcAt(clientX: number, clientY: number): Arc | null {
    if (!this.geometry) return null
    const rect = this.canvas.getBoundingClientRect()
    const x = ((clientX - rect.left) / rect.width) * this.canvas.clientWidth
    const y = ((clientY - rect.top) / rect.height) * this.canvas.clientHeight
    return hitTest(this.geometry, x, y)
  }

  isCentre(clientX: number, clientY: number): boolean {
    if (!this.geometry) return false
    const rect = this.canvas.getBoundingClientRect()
    const x = ((clientX - rect.left) / rect.width) * this.canvas.clientWidth
    const y = ((clientY - rect.top) / rect.height) * this.canvas.clientHeight
    return Math.hypot(x - this.geometry.cx, y - this.geometry.cy) < this.geometry.inner
  }

  resize(): void {
    const ratio = window.devicePixelRatio || 1
    const rect = this.canvas.getBoundingClientRect()
    const width = Math.max(320, Math.round(rect.width))
    const height = Math.max(320, Math.round(rect.height))
    this.canvas.width = Math.round(width * ratio)
    this.canvas.height = Math.round(height * ratio)
    this.context.setTransform(ratio, 0, 0, ratio, 0, 0)
    this.relayout()
    this.draw()
  }

  private relayout(): void {
    const width = this.canvas.clientWidth
    const height = this.canvas.clientHeight
    this.geometry = computeGeometry(this.view, width, height)
  }

  draw(): void {
    const { context: ctx, geometry, view } = this
    const width = this.canvas.clientWidth
    const height = this.canvas.clientHeight
    ctx.clearRect(0, 0, width, height)
    if (!geometry || view.length === 0) return

    const background = getComputedStyle(this.canvas).getPropertyValue('--stage-bg').trim() || '#1c1c1e'
    ctx.save()
    ctx.fillStyle = background
    ctx.fillRect(0, 0, width, height)

    // Petals, outermost first so inner rings sit on top of any seam.
    for (const arc of geometry.arcs) {
      const node = view[arc.node]
      if (!node) continue
      const inner = radiusOf(geometry, arc.depth - 1)
      const outer = radiusOf(geometry, arc.depth)
      const hue = this.hues[arc.node] ?? 210

      ctx.beginPath()
      ctx.arc(geometry.cx, geometry.cy, outer, arc.start - Math.PI / 2, arc.end - Math.PI / 2)
      ctx.arc(geometry.cx, geometry.cy, inner, arc.end - Math.PI / 2, arc.start - Math.PI / 2, true)
      ctx.closePath()

      const isHovered = this.hovered?.node === arc.node && this.hovered.depth === arc.depth
      ctx.fillStyle = isHovered ? 'hsl(0 0% 100%)' : colorFor(node, hue, arc.depth)
      ctx.fill()

      ctx.lineWidth = isHovered ? 2 : 1
      ctx.strokeStyle = isHovered ? '#ffffff' : 'rgba(20, 20, 22, 0.55)'
      ctx.stroke()

      if (node.node_id !== null && this.collected.has(node.node_id) && !isHovered) {
        ctx.save()
        ctx.globalAlpha = 0.85
        ctx.strokeStyle = '#ffd60a'
        ctx.lineWidth = 2
        ctx.stroke()
        ctx.restore()
      }
    }

    // The centre is a button, not a slice: it climbs one level.
    const root = view[0]
    ctx.beginPath()
    ctx.arc(geometry.cx, geometry.cy, geometry.inner - 2, 0, Math.PI * 2)
    ctx.fillStyle = '#0f0f11'
    ctx.fill()
    ctx.strokeStyle = 'rgba(255, 255, 255, 0.12)'
    ctx.lineWidth = 1
    ctx.stroke()

    if (root) {
      ctx.fillStyle = '#f5f5f7'
      ctx.textAlign = 'center'
      ctx.textBaseline = 'middle'
      const size = Math.max(11, geometry.inner * 0.26)
      const [value, unit] = formatBytes(root.bytes).split(' ')
      ctx.font = `500 ${size}px -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif`
      ctx.fillText(value ?? '', geometry.cx, geometry.cy - size * 0.45)
      ctx.font = `500 ${size * 0.62}px -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif`
      ctx.fillStyle = 'rgba(245, 245, 247, 0.62)'
      ctx.fillText(unit ?? '', geometry.cx, geometry.cy + size * 0.55)
    }

    ctx.restore()
  }
}
