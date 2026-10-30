import './style.css'
import * as bridge from './bridge'
import { Sunburst } from './sunburst'
import { assignHues, colorFor } from './layout'
import type { Arc } from './layout'
import { formatBytes, formatCount } from './format'
import type { ViewNode } from './types'
import { applyBranch, advance, findChildByName, partialView, startBranches } from './partial'
import type { PartialBranch } from './partial'

function el<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id)
  if (!node) throw new Error(`missing #${id}`)
  return node as T
}

const canvas = el<HTMLCanvasElement>('sunburst')
const crumbs = el<HTMLElement>('crumbs')
const status = el<HTMLElement>('status')
const legend = el<HTMLElement>('legend')
const collectorBar = el<HTMLElement>('collector')
const collectAmount = el<HTMLElement>('collect-amount')
const collectCaption = el<HTMLElement>('collect-caption')
const collectValue = el<HTMLElement>('collect-value')
const deleteButton = el<HTMLButtonElement>('delete-button')
const hoverLabel = el<HTMLElement>('hover-label')

const sunburst = new Sunburst(canvas)

// ─── progressive paint ────────────────────────────────────────────────────────────────
//
// The pure half of this lives in partial.ts, where it is tested: petals start as equal
// "measuring" petals and grow to their real width as the scanner reports each branch.

let branches: PartialBranch[] | null = null
// A branch clicked while it was still measuring. The scan walks every branch of the root
// concurrently, so there is no queue to jump: the honest meaning of the click is "open
// this as soon as its report lands".
let pendingDrill: string | null = null
let partialFrame = 0
let partialRootName = ''

function startPartial(names: string[], rootName: string): void {
  partialRootName = rootName
  pendingDrill = null
  branches = startBranches(names)
  sunburst.setView(partialView(partialRootName, branches))
}

function updatePartial(name: string, bytes: number): void {
  if (!branches) return
  if (!applyBranch(branches, name, bytes)) return
  setStatus(`Measuring… ${branches.filter((b) => b.done).length} of ${branches.length} done`)
  animatePartial()
}

function animatePartial(): void {
  if (partialFrame) return
  const step = (): void => {
    if (!branches) {
      partialFrame = 0
      return
    }
    const moving = advance(branches)
    sunburst.setView(partialView(partialRootName, branches))
    partialFrame = moving ? requestAnimationFrame(step) : 0
  }
  partialFrame = requestAnimationFrame(step)
}

function stopPartial(): void {
  branches = null
  if (partialFrame) {
    cancelAnimationFrame(partialFrame)
    partialFrame = 0
  }
}

interface Frame {
  nodeId: number | null
  view: ViewNode[]
}

const state = {
  scanId: '',
  rootPath: '',
  stack: [] as Frame[],
  collectedTotal: 0,
  collectedCount: 0,
  collectedIds: [] as number[],
  needsConfirmation: false,
}

function setStatus(text: string): void {
  status.textContent = text
}

async function scanPath(path: string): Promise<void> {
  setStatus('Scanning…')
  document.body.classList.add('scanning')
  let unsubscribe: (() => void) | null = null
  try {
    const names = await bridge.peekRoot(path)
    startPartial(names, path.split('/').filter(Boolean).pop() ?? path)
    unsubscribe = await bridge.onScanProgress((branch) => updatePartial(branch.name, branch.bytes))

    const result = await bridge.scan(path)
    stopPartial()
    state.scanId = result.scan_id
    state.rootPath = path
    state.stack = [{ nodeId: null, view: result.view }]
    state.collectedTotal = 0
    state.collectedCount = 0
    state.collectedIds = []
    sunburst.setView(result.view)
    sunburst.setCollected([])
    renderCrumbs()
    renderLegend()
    updateCollector()

    // Honour a click made while the petal was still measuring.
    const wanted = pendingDrill
    pendingDrill = null
    if (wanted) {
      const target = findChildByName(result.view, wanted)
      if (target !== null) void navigate(target)
    }
    const restrictedNote = result.partial ? ' · some items restricted' : ''
    setStatus(
      `${formatBytes(result.bytes)} · ${formatCount(result.files)} files · ${(
        result.duration_ms / 1000
      ).toFixed(1)}s${restrictedNote}`,
    )
  } catch (error) {
    stopPartial()
    setStatus(`Scan failed: ${String(error)}`)
  } finally {
    unsubscribe?.()
    document.body.classList.remove('scanning')
  }
}

async function navigate(nodeId: number): Promise<void> {
  try {
    const view = await bridge.view(state.scanId, nodeId)
    state.stack.push({ nodeId, view })
    sunburst.setView(view)
    renderCrumbs()
    renderLegend()
    updateHoverLabel(null)
  } catch (error) {
    setStatus(`Could not open that item: ${String(error)}`)
  }
}

function up(): void {
  if (state.stack.length <= 1) return
  state.stack.pop()
  const frame = state.stack[state.stack.length - 1]
  if (!frame) return
  sunburst.setView(frame.view)
  renderCrumbs()
  renderLegend()
  updateHoverLabel(null)
}

function current(): ViewNode | undefined {
  return state.stack[state.stack.length - 1]?.view[0]
}

function renderCrumbs(): void {
  crumbs.replaceChildren()
  const trail = state.stack
    .map((frame) => frame.view[0]?.name)
    .filter((name): name is string => Boolean(name))

  if (trail.length === 0) {
    const fallback = document.createElement('span')
    fallback.className = 'crumb active'
    fallback.textContent = state.rootPath || 'Nothing scanned'
    crumbs.append(fallback)
    return
  }

  trail.forEach((name, i) => {
    const crumb = document.createElement('button')
    crumb.type = 'button'
    crumb.className = i === trail.length - 1 ? 'crumb active' : 'crumb'
    crumb.textContent = name
    crumb.addEventListener('click', () => {
      while (state.stack.length > i + 1) up()
    })
    crumbs.append(crumb)
  })

  const scanButton = document.createElement('button')
  scanButton.type = 'button'
  scanButton.className = 'crumb action'
  scanButton.textContent = 'Rescan'
  scanButton.addEventListener('click', () => {
    if (state.rootPath) void scanPath(state.rootPath)
  })
  crumbs.append(scanButton)
}

function renderLegend(): void {
  legend.replaceChildren()
  const frame = state.stack[state.stack.length - 1]
  const root = frame?.view[0]
  if (!frame || !root) return

  const view = frame.view
  const hues = assignHues(view)
  const heading = document.createElement('div')
  heading.className = 'legend-heading'
  heading.innerHTML = `<span>${root.name}</span><strong>${formatBytes(root.bytes)}</strong>`
  legend.append(heading)

  const list = document.createElement('div')
  list.className = 'legend-list'

  const children = [...root.children].sort((a, b) => (view[b]?.bytes ?? 0) - (view[a]?.bytes ?? 0))
  for (const index of children) {
    const node = view[index]
    if (!node) continue

    const row = document.createElement('div')
    row.className = 'legend-row'
    row.tabIndex = 0

    const dot = document.createElement('span')
    dot.className = 'dot'
    dot.style.background = colorFor(node, hues[index] ?? 210, 1)

    const name = document.createElement('span')
    name.className = 'legend-name'
    name.textContent = node.kind === 'aggregate' ? `${node.name} (${node.aggregated_count})` : node.name

    const size = document.createElement('span')
    size.className = 'legend-size'
    size.textContent = formatBytes(node.bytes)

    row.append(dot, name, size)

    if (node.node_id !== null) {
      const collectButton = document.createElement('button')
      const nodeId = node.node_id
      const collected = state.collectedIds.includes(nodeId)
      collectButton.type = 'button'
      collectButton.className = collected ? 'collect on' : 'collect'
      collectButton.title = collected ? 'Remove from collector' : 'Add to collector'
      collectButton.textContent = collected ? '✓' : '+'
      collectButton.addEventListener('click', (event) => {
        event.stopPropagation()
        void toggleCollect(nodeId, !collected)
      })
      row.append(collectButton)

      row.addEventListener('click', () => void navigate(nodeId))
      row.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') void navigate(nodeId)
      })
    }

    list.append(row)
  }

  legend.append(list)
}

async function toggleCollect(nodeId: number, add: boolean): Promise<void> {
  try {
    const result = await bridge.collect(state.scanId, nodeId, add)
    state.collectedTotal = result.total
    state.collectedCount = result.count
    state.collectedIds = result.ids
    state.needsConfirmation = result.needs_confirmation
    sunburst.setCollected(result.ids)
    updateCollector()
    renderLegend()
  } catch (error) {
    setStatus(`Could not update the collector: ${String(error)}`)
  }
}

function updateCollector(): void {
  collectAmount.textContent = formatBytes(state.collectedTotal)
  collectCaption.textContent =
    state.collectedCount === 0
      ? 'in the collector · drag petals here'
      : state.collectedCount === 1
        ? 'in the collector · 1 item'
        : `in the collector · ${state.collectedCount} items`
  collectValue.textContent = state.collectedTotal > 0 ? formatBytes(state.collectedTotal).split(' ')[0] ?? '0' : '0'
  deleteButton.disabled = state.collectedCount === 0
}

function updateHoverLabel(arc: Arc | null): void {
  const node = sunburst.nodeAt(arc)
  if (!node) {
    hoverLabel.hidden = true
    return
  }
  hoverLabel.hidden = false
  const kind =
    node.kind === 'restricted'
      ? 'restricted'
      : node.kind === 'aggregate'
        ? `${node.aggregated_count} items`
        : node.kind === 'file'
          ? 'file'
          : 'folder'
  hoverLabel.innerHTML = `<strong>${node.name}</strong><span>${formatBytes(node.bytes)} · ${kind}</span>`
}

async function onDelete(permanent: boolean): Promise<void> {
  if (state.collectedCount === 0) return

  const verb = permanent ? 'Delete permanently' : 'Move to the Trash'
  const warning = permanent
    ? `${verb} ${formatBytes(state.collectedTotal)}?\n\nThis cannot be undone.`
    : `${verb} ${formatBytes(state.collectedTotal)}?\n\nYou can recover them until the Trash is emptied.`

  // Rust decides whether this needs confirming: a top-level folder asks whatever its size,
  // and anything over the size bound asks. Permanent deletion always asks.
  if (permanent || state.needsConfirmation) {
    if (!window.confirm(warning)) return
  }

  const frame = state.stack[state.stack.length - 1]
  const nodeId = frame?.nodeId ?? null

  try {
    const report = await bridge.deleteCollected(state.scanId, nodeId, permanent, true)

    // The backend pruned the tree and handed back a fresh view, so no rescan is needed.
    if (frame) frame.view = report.view
    sunburst.setView(report.view)
    sunburst.setCollected([])
    state.collectedTotal = 0
    state.collectedCount = 0
    state.collectedIds = []
    state.needsConfirmation = false
    updateCollector()
    renderLegend()

    const failed = report.failed > 0 ? `, ${report.failed} failed` : ''
    setStatus(
      `${permanent ? 'Deleted' : 'Moved to Trash'} ${formatBytes(report.freed)} · ${report.removed} item${report.removed === 1 ? '' : 's'}${failed}`,
    )
  } catch (error) {
    setStatus(`Delete failed: ${String(error)}`)
  }
}

// ─── input ────────────────────────────────────────────────────────────────────────────

let press: { arc: Arc | null; x: number; y: number } | null = null
let dragging = false

canvas.addEventListener('pointerdown', (event) => {
  press = { arc: sunburst.arcAt(event.clientX, event.clientY), x: event.clientX, y: event.clientY }
  dragging = false
  canvas.setPointerCapture(event.pointerId)
})

canvas.addEventListener('pointermove', (event) => {
  const arc = sunburst.arcAt(event.clientX, event.clientY)
  sunburst.setHovered(arc)
  updateHoverLabel(arc)

  if (press && !dragging && press.arc && Math.hypot(event.clientX - press.x, event.clientY - press.y) > 8) {
    dragging = true
    document.body.classList.add('dragging')
  }

  if (dragging) {
    const over = collectorBar.getBoundingClientRect()
    const inside =
      event.clientY >= over.top && event.clientY <= over.bottom && event.clientX >= over.left && event.clientX <= over.right
    collectorBar.classList.toggle('armed', inside)
  }
})

canvas.addEventListener('pointerup', (event) => {
  const wasDragging = dragging
  const start = press
  press = null
  dragging = false
  document.body.classList.remove('dragging')
  collectorBar.classList.remove('armed')

  if (wasDragging && start?.arc) {
    const over = collectorBar.getBoundingClientRect()
    const inside =
      event.clientY >= over.top && event.clientY <= over.bottom && event.clientX >= over.left && event.clientX <= over.right
    const node = sunburst.nodeAt(start.arc)
    if (inside && node?.node_id != null) {
      void toggleCollect(node.node_id, true)
    }
    return
  }

  if (sunburst.isCentre(event.clientX, event.clientY)) {
    up()
    return
  }

  const arc = sunburst.arcAt(event.clientX, event.clientY)
  const node = sunburst.nodeAt(arc)

  // Measuring phase: nothing has an id yet, so remember the choice and open it on arrival.
  if (branches && node) {
    pendingDrill = node.name
    setStatus(`Opening ${node.name} as soon as it is measured…`)
    return
  }

  if (node?.node_id != null) void navigate(node.node_id)
})

canvas.addEventListener('pointerleave', () => {
  sunburst.setHovered(null)
  updateHoverLabel(null)
})

canvas.addEventListener('dblclick', (event) => {
  void event
})

window.addEventListener('keydown', (event) => {
  const hovered = sunburst.nodeAt(sunburst.getHovered())

  if (event.key === ' ' && hovered?.node_id != null) {
    event.preventDefault()
    void bridge.quicklook(state.scanId, hovered.node_id).catch((error: unknown) => {
      setStatus(`Quick Look failed: ${String(error)}`)
    })
    return
  }

  if ((event.key === 'Backspace' || event.key === 'Delete') && hovered?.node_id != null) {
    event.preventDefault()
    void toggleCollect(hovered.node_id, !state.collectedIds.includes(hovered.node_id))
    return
  }

  if (event.key === 'Escape') {
    if (pendingDrill) {
      pendingDrill = null
      setStatus('Cancelled')
      return
    }
    up()
  }

  if (event.key === 'ArrowUp') up()
})

deleteButton.addEventListener('click', (event) => void onDelete(event.altKey))
// Alt-click is the permanent path; the button's tooltip says so.
deleteButton.title = 'Move to Trash · hold Option to delete permanently'

const resizeObserver = new ResizeObserver(() => sunburst.resize())
resizeObserver.observe(canvas)

// ─── boot ─────────────────────────────────────────────────────────────────────────────

async function boot(): Promise<void> {
  try {
    const [home, volumes] = await Promise.all([bridge.homeDir(), bridge.volumes()])
    if (volumes[0]) {
      setStatus(`${volumes[0].name} · ${formatBytes(volumes[0].free_bytes)} free`)
    }
    await scanPath(home)
  } catch (error) {
    setStatus(`Startup failed: ${String(error)}`)
  }
}

void boot()

// Exposed for the screenshot harness and for manual poking in a devtools console.
declare global {
  interface Window {
    diskguardian?: {
      state: typeof state
      scanPath: typeof scanPath
      navigate: typeof navigate
      current: typeof current
    }
  }
}

window.diskguardian = { state, scanPath, navigate, current }
