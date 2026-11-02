import './style.css'
import * as bridge from './bridge'
import { Sunburst } from './sunburst'
import { assignHues, colorFor } from './layout'
import type { Arc } from './layout'
import { formatBytes, formatCount } from './format'
import type { CleanupItem, ViewNode, VolumeInfo } from './types'
import { basename, gaugeFor, toneColor } from './overview'
import { applyBranch, advance, findChildByName, partialView, startBranches } from './partial'
import type { PartialBranch } from './partial'
import { ADMIN_REFUSED_STATUS, adminStatus, isMacPlatform } from './admin'
import { duplicatesModel } from './duplicates'
import type { DuplicateGroupModel } from './duplicates'

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
const toast = el<HTMLElement>('toast')
const toastText = el<HTMLElement>('toast-text')
const overview = el<HTMLElement>('overview')
const volumeList = el<HTMLElement>('volume-list')
const splitter = el<HTMLElement>('splitter')
const cleanupPanel = el<HTMLElement>('cleanup')
const candidatesList = el<HTMLElement>('candidates-list')
const biggestList = el<HTMLElement>('biggest-list')
const adminAction = el<HTMLButtonElement>('admin-action')
const duplicatesPanel = el<HTMLElement>('duplicates')
const duplicatesHeadline = el<HTMLElement>('duplicates-headline')
const duplicatesNote = el<HTMLElement>('duplicates-note')
const duplicatesList = el<HTMLElement>('duplicates-list')

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
  // Nothing has an id yet, so nothing can be collected: say so rather than showing a
  // dead button with no explanation.
  collectAmount.textContent = '0 bytes'
  collectCaption.textContent = 'measuring the disk · collecting opens when the scan finishes'
  deleteButton.disabled = true
  deleteButton.title = 'Wait for the scan to finish: items are collectable once they are measured'
  sunburst.setView(partialView(partialRootName, branches))
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

// ─── administrator measurement ───────────────────────────────────────────────────────
//
// Offered after a partial scan on macOS: one OS consent dialog, then the purple
// petals are re-measured and grafted server-side. The frontend only ever sends the
// scan id; it cannot name a path.

let adminBeforeBytes = 0

function offerAdminMeasurement(beforeBytes: number): void {
  adminBeforeBytes = beforeBytes
  adminAction.disabled = false
  adminAction.hidden = false
}

function hideAdminMeasurement(): void {
  adminAction.hidden = true
  adminAction.disabled = false
}

async function measureRestricted(): Promise<void> {
  if (!state.scanId) return
  adminAction.disabled = true
  setStatus('Measuring restricted areas… the system will ask for your administrator password')
  const unsubscribe = await bridge.onAdminProgress((p) =>
    setStatus(`Measuring… ${p.measured} of ${p.total} areas done`),
  )
  try {
    const report = await bridge.scanRestricted(state.scanId)
    unsubscribe()
    // The backend grafted the measured subtrees and returned a fresh whole-tree view.
    state.stack = [{ nodeId: null, view: report.view }]
    sunburst.setView(report.view)
    renderCrumbs()
    renderLegend()
    setStatus(adminStatus(Math.max(0, report.bytes - adminBeforeBytes), report.still_restricted))
    hideAdminMeasurement()
  } catch (error) {
    unsubscribe()
    const message = String(error)
    adminAction.disabled = false
    if (message.includes('declined')) {
      setStatus(ADMIN_REFUSED_STATUS)
    } else {
      setStatus(`Administrator measurement failed: ${message}`)
    }
  }
}

adminAction.addEventListener('click', () => void measureRestricted())

/**
 * Something the user must see. The first version of this reported failures only in the
 * status line, which made a broken control look identical to a working one.
 */
function showError(text: string): void {
  console.error(text)
  toastText.textContent = text
  toast.hidden = false
  setStatus(text)
}

async function scanPath(path: string): Promise<void> {
  overview.hidden = true
  // Both panels describe the scan that was on screen; a new scan makes them stale.
  cleanupPanel.hidden = true
  duplicatesPanel.hidden = true
  hideAdminMeasurement()
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

    // Probe the collector IPC straight away: if a multi-argument command is miswired, that
    // is visible now rather than the first time somebody presses +.
    try {
      const existing = await bridge.collected(result.scan_id)
      state.collectedTotal = existing.total
      state.collectedCount = existing.count
      state.collectedIds = existing.ids
      state.needsConfirmation = existing.needs_confirmation
      updateCollector()
    } catch (error) {
      showError(`Collector IPC failed: ${String(error)}`)
    }

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
    // The affordance appears only where a runner exists (macOS) and only when there
    // is something to measure.
    if (result.partial && isMacPlatform(navigator.userAgent)) {
      // Already authorised in this run: measure straight away and do not offer a second
      // prompt the user has effectively already given.
      const authorised = await bridge.adminAuthorised().catch(() => false)
      if (authorised) {
        state.stack = [{ nodeId: null, view: result.view }]
        void measureRestricted()
      } else {
        offerAdminMeasurement(result.bytes)
      }
    }
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

  const back = document.createElement('button')
  back.type = 'button'
  back.className = 'crumb'
  back.textContent = 'Disks and folders'
  back.addEventListener('click', () => void showOverview())
  crumbs.prepend(back)

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

  // Say the purple petals out loud: the map shows them, but a count with the reason is what
  // tells the user whether an administrator pass or Full Disk Access would help.
  const restrictedCount = children.filter((i) => view[i]?.kind === 'restricted').length
  if (restrictedCount > 0) {
    const note = document.createElement('div')
    note.className = 'legend-note'
    note.textContent = `${restrictedCount} area${restrictedCount === 1 ? '' : 's'} not measurable`
    note.title =
      'Purple petals. macOS privacy controls (TCC) do not yield to an administrator pass; grant Full Disk Access to read these.'
    legend.append(note)
  }

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
    const label = node.kind === 'aggregate' ? `${node.name} (${node.aggregated_count})` : node.name
    name.textContent = label
    // The column is narrow and names are long: never leave the full name unreachable.
    name.title = label
    row.title = node.missing
      ? `${label} — deleted, rescan to clear`
      : node.kind === 'symlink'
        ? `${label} — symbolic link: points elsewhere and costs only the link itself. Not offered for collection.`
        : node.kind === 'restricted'
          ? `${label} — restricted: the system will not let any app read this. Measure restricted areas, or grant Full Disk Access, to see a size.`
          : label
    if (node.missing) {
      name.classList.add('missing')
      row.classList.add('missing')
    }

    const size = document.createElement('span')
    size.className = 'legend-size'
    size.textContent = node.missing
      ? 'deleted'
      : node.kind === 'restricted'
        ? '🔒 unmeasured'
        : node.kind === 'symlink'
          ? '→ link'
          : formatBytes(node.bytes)

    row.append(dot, name, size)

    // A symlink is a pointer someone placed on purpose: never offered for collection.
    if (node.node_id !== null && node.kind !== 'symlink') {
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

/** Returns whether the collector actually changed, so a caller can draw a tick that is true. */
async function toggleCollect(nodeId: number, add: boolean): Promise<boolean> {
  try {
    const result = await bridge.collect(state.scanId, nodeId, add)
    state.collectedTotal = result.total
    state.collectedCount = result.count
    state.collectedIds = result.ids
    state.needsConfirmation = result.needs_confirmation
    sunburst.setCollected(result.ids)
    updateCollector()
    renderLegend()
    return true
  } catch (error) {
    showError(`Could not update the collector: ${String(error)}`)
    return false
  }
}

function updateCollector(): void {
  collectAmount.textContent = formatBytes(state.collectedTotal)
  collectCaption.textContent =
    state.collectedCount === 0
      ? 'drag a petal here, click + in the list, or hover a petal and press ⌫ · double-click to reveal, Space to preview'
      : state.collectedCount === 1
        ? 'in the collector · 1 item'
        : `in the collector · ${state.collectedCount} items`
  collectValue.textContent = state.collectedTotal > 0 ? formatBytes(state.collectedTotal).split(' ')[0] ?? '0' : '0'
  deleteButton.disabled = state.collectedCount === 0
  deleteButton.title =
    state.collectedCount === 0
      ? 'Collect something first: click + in the list, drag a petal here, or press ⌫ on a highlighted petal'
      : 'Move to Trash · hold Option to delete permanently'
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
  hoverLabel.title = node.name
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
  const node = sunburst.nodeAt(sunburst.arcAt(event.clientX, event.clientY))
  if (node?.node_id == null) return
  void bridge.reveal(state.scanId, node.node_id).catch((error: unknown) => {
    showError(`Could not reveal in Finder: ${String(error)}`)
  })
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

  // Cmd-L focuses the path field, the way a browser's location bar does.
  if (event.key === 'l' && (event.metaKey || event.ctrlKey) && !overview.hidden) {
    event.preventDefault()
    el<HTMLInputElement>('manual-path').focus()
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
el<HTMLButtonElement>('toast-close').addEventListener('click', () => {
  toast.hidden = true
})
// Alt-click is the permanent path; the button's tooltip says so.
deleteButton.title = 'Move to Trash · hold Option to delete permanently'

const resizeObserver = new ResizeObserver(() => sunburst.resize())
resizeObserver.observe(canvas)

// ─── resizable legend ────────────────────────────────────────────────────────────────
//
// The contents list is the place long names and deep folders live, so its width is the
// user's call. The width persists, double-click resets it, and arrow keys nudge it for
// anyone who cannot drag.

const LEGEND_MIN = 220
const LEGEND_MAX = 640
const LEGEND_DEFAULT = 320

function setLegendWidth(width: number): void {
  const clamped = Math.min(LEGEND_MAX, Math.max(LEGEND_MIN, Math.round(width)))
  document.documentElement.style.setProperty('--legend-width', `${clamped}px`)
  try {
    localStorage.setItem('diskguardian.legendWidth', String(clamped))
  } catch {
    // A private window with storage disabled is not a reason to lose the resize.
  }
}

function legendWidth(): number {
  const stored = Number(localStorage.getItem('diskguardian.legendWidth'))
  return Number.isFinite(stored) && stored > 0 ? stored : LEGEND_DEFAULT
}

try {
  setLegendWidth(legendWidth())
} catch {
  setLegendWidth(LEGEND_DEFAULT)
}

splitter.addEventListener('pointerdown', (event) => {
  event.preventDefault()
  splitter.setPointerCapture(event.pointerId)
  splitter.classList.add('dragging')
  const startX = event.clientX
  const startWidth = legendWidth()

  const move = (moveEvent: PointerEvent): void => {
    setLegendWidth(startWidth - (moveEvent.clientX - startX))
  }
  const stop = (): void => {
    splitter.classList.remove('dragging')
    splitter.removeEventListener('pointermove', move)
    splitter.removeEventListener('pointerup', stop)
    sunburst.resize()
  }

  splitter.addEventListener('pointermove', move)
  splitter.addEventListener('pointerup', stop)
})

splitter.addEventListener('dblclick', () => {
  setLegendWidth(LEGEND_DEFAULT)
  sunburst.resize()
})

splitter.addEventListener('keydown', (event) => {
  if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
  event.preventDefault()
  setLegendWidth(legendWidth() + (event.key === 'ArrowLeft' ? 20 : -20))
  sunburst.resize()
})

// ─── boot ─────────────────────────────────────────────────────────────────────────────

let homePath = ''

/** The landing screen: pick a volume, a folder, or drop something on the window. */
async function showOverview(): Promise<void> {
  overview.hidden = false
  hoverLabel.hidden = true
  hideAdminMeasurement()
  setStatus('Choose a disk or folder to scan')

  let volumes: VolumeInfo[] = []
  try {
    volumes = await bridge.volumes()
  } catch (error) {
    showError(`Could not list volumes: ${String(error)}`)
  }

  volumeList.replaceChildren()

  // The home folder is the most common target after the whole disk, so it gets its own row.
  if (homePath) {
    volumeList.append(
      volumeRow({
        name: 'Home folder',
        mount: homePath,
        total_bytes: 0,
        free_bytes: 0,
        detail: basename(homePath),
        gauge: false,
      }),
    )
  }

  // Anything already in the trash is still occupying the disk, and is the cheapest win on
  // offer. Show the number before offering the button.
  try {
    const trash = await bridge.trashStats()
    if (trash.supported && trash.items > 0) {
      volumeList.append(
        volumeRow({
          name: 'Trash',
          mount: trash.path,
          total_bytes: trash.bytes,
          free_bytes: 0,
          detail: `${formatBytes(trash.bytes)} · ${trash.items} item${trash.items === 1 ? '' : 's'} · emptying this is the only way to get it back`,
          gauge: false,
          actionLabel: 'Empty',
          actionTone: 'var(--danger)',
          onAction: () => void emptyTrashNow(trash.bytes),
        }),
      )
    }
  } catch (error) {
    console.error('trash stats failed', error)
  }

  for (const volume of volumes) {
    const gauge = gaugeFor(volume)
    volumeList.append(
      volumeRow({
        name: volume.name,
        mount: volume.mount,
        total_bytes: volume.total_bytes,
        free_bytes: volume.free_bytes,
        detail: `${gauge.freeLabel} of ${formatBytes(volume.total_bytes)}`,
        gauge: true,
        usedRatio: gauge.usedRatio,
        tone: toneColor(gauge.tone),
      }),
    )
  }
}

interface VolumeRowOptions {
  name: string
  mount: string
  total_bytes: number
  free_bytes: number
  detail: string
  gauge: boolean
  usedRatio?: number
  tone?: string
  /** Defaults to Scan, which scans `mount`. */
  actionLabel?: string
  actionTone?: string
  onAction?: () => void
}

function volumeRow(options: VolumeRowOptions): HTMLElement {
  const row = document.createElement('div')
  row.className = 'volume-row'

  const left = document.createElement('div')
  const name = document.createElement('div')
  name.className = 'volume-name'
  name.textContent = options.name

  const meta = document.createElement('div')
  meta.className = 'volume-meta'
  meta.textContent = options.detail

  left.append(name, meta)

  if (options.gauge) {
    const bar = document.createElement('div')
    bar.className = 'volume-bar'
    const fill = document.createElement('span')
    fill.style.width = `${Math.round((options.usedRatio ?? 0) * 100)}%`
    fill.style.background = options.tone ?? '#30d158'
    bar.append(fill)
    left.append(bar)
  }

  const scan = document.createElement('button')
  scan.type = 'button'
  scan.className = 'volume-scan'
  scan.textContent = options.actionLabel ?? 'Scan'
  if (options.actionTone) scan.style.background = options.actionTone
  scan.addEventListener('click', () => {
    if (options.onAction) options.onAction()
    else void scanPath(options.mount)
  })

  row.append(left, scan)
  return row
}

async function boot(): Promise<void> {
  try {
    homePath = await bridge.homeDir()
    await showOverview()
  } catch (error) {
    showError(`Startup failed: ${String(error)}`)
  }

  el<HTMLButtonElement>('scan-folder').addEventListener('click', () => void pickFolder())
  el<HTMLButtonElement>('cleanup-open').addEventListener('click', () => {
    duplicatesPanel.hidden = true
    cleanupPanel.hidden = false
    void refreshCleanup()
  })
  el<HTMLButtonElement>('duplicates-open').addEventListener('click', () => {
    cleanupPanel.hidden = true
    duplicatesPanel.hidden = false
    void refreshDuplicates()
  })
  el<HTMLButtonElement>('duplicates-close').addEventListener('click', () => {
    duplicatesPanel.hidden = true
  })
  el<HTMLButtonElement>('duplicates-run').addEventListener('click', () => void refreshDuplicates())
  el<HTMLSelectElement>('duplicates-floor').addEventListener('change', () => void refreshDuplicates())
  el<HTMLButtonElement>('cleanup-close').addEventListener('click', () => {
    cleanupPanel.hidden = true
  })
  el<HTMLButtonElement>('cleanup-run').addEventListener('click', () => void refreshCleanup())
  el<HTMLSelectElement>('cleanup-floor').addEventListener('change', () => void refreshCleanup())
  el<HTMLSelectElement>('cleanup-age').addEventListener('change', () => void refreshCleanup())

  el<HTMLFormElement>('manual-path-form').addEventListener('submit', (event) => {
    event.preventDefault()
    void scanTypedPath()
  })

  // The tree is a snapshot; deletions made outside the app are invisible until something
  // re-reads the disk. Regaining focus is the moment a person expects it to be current.
  if (bridge.isTauri) {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) void refreshIfStale()
    })
  }

  // Drop a disk or folder on the window, the way DaisyDisk does it.
  if (bridge.isTauri) {
    const { getCurrentWebview } = await import('@tauri-apps/api/webview')
    await getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'drop') {
        const dropped = event.payload.paths[0]
        if (dropped) {
          overview.hidden = true
          void scanPath(dropped)
        }
      }
    })
  }
}

let lastRefresh = 0

/** Re-stat the current level and prune anything that has disappeared. */
async function refreshIfStale(): Promise<void> {
  if (!state.scanId || branches) return
  if (Date.now() - lastRefresh < 4000) return
  lastRefresh = Date.now()

  const frame = state.stack[state.stack.length - 1]
  if (!frame) return

  try {
    const report = await bridge.revalidate(state.scanId, frame.nodeId)
    if (report.removed === 0) return

    frame.view = report.view
    sunburst.setView(report.view)
    renderLegend()
    setStatus(
      `${report.removed} item${report.removed === 1 ? '' : 's'} no longer on disk · ${formatBytes(report.bytes)}`,
    )
  } catch (error) {
    showError(`Could not refresh the view: ${String(error)}`)
  }
}

/** Emptying is irreversible and global, so it names the size first and always confirms. */
async function emptyTrashNow(bytes: number): Promise<void> {
  const ok = window.confirm(
    `Empty the Trash?\n\nThis frees ${formatBytes(bytes)} and cannot be undone.`,
  )
  if (!ok) return

  try {
    const report = await bridge.emptyTrash(true)
    const failed = report.failed > 0 ? `, ${report.failed} could not be removed` : ''
    setStatus(`Freed ${formatBytes(report.freed)} · ${report.removed} item(s) removed${failed}`)
    await showOverview()
  } catch (error) {
    showError(`Could not empty the Trash: ${String(error)}`)
  }
}

// ─── cleanup ──────────────────────────────────────────────────────────────────────────

function ageLabel(modified: number | null): string {
  if (modified === null) return 'age unknown'
  const days = Math.max(0, Math.floor((Date.now() / 1000 - modified) / 86_400))
  if (days === 0) return 'today'
  if (days === 1) return 'yesterday'
  if (days < 30) return `${days} days old`
  if (days < 365) return `${Math.floor(days / 30)} months old`
  return `${Math.floor(days / 365)} years old`
}

function cleanupRow(item: CleanupItem, onCollect: () => void): HTMLElement {
  const row = document.createElement('div')
  row.className = 'cleanup-row'

  const left = document.createElement('div')
  const name = document.createElement('div')
  name.className = 'cleanup-name'
  name.textContent = item.path.split('/').slice(-1)[0] ?? item.path
  name.title = item.path

  const why = document.createElement('div')
  why.className = 'cleanup-why'
  why.textContent = `${item.reason} · ${ageLabel(item.newest_modified)} · ${item.path}`
  why.title = item.path

  left.append(name, why)

  const size = document.createElement('span')
  size.className = 'cleanup-size'
  size.textContent = formatBytes(item.bytes)

  const add = document.createElement('button')
  add.type = 'button'
  add.className = 'cleanup-add'
  add.textContent = '+'
  add.title = 'Add to the collector'
  add.addEventListener('click', onCollect)

  row.append(left, size, add)
  return row
}

async function refreshCleanup(): Promise<void> {
  const frame = state.stack[state.stack.length - 1]
  if (!state.scanId || !frame) {
    setStatus('Scan something first')
    return
  }

  const floor = Number(el<HTMLSelectElement>('cleanup-floor').value)
  const ageValue = el<HTMLSelectElement>('cleanup-age').value
  const [daysAxis] = ageValue.split(':')
  const ageDays = ageValue === '' ? null : Number(daysAxis)
  const newer = ageValue.endsWith(':new')

  candidatesList.replaceChildren()
  biggestList.replaceChildren()
  setStatus('Working out what is filling the disk…')

  try {
    const report = await bridge.cleanup(state.scanId, floor, ageDays, newer)

    el<HTMLElement>('candidates-title').textContent =
      `Candidates · ${report.candidates.length} · ${formatBytes(report.total_candidate_bytes)}`

    if (report.candidates.length === 0) {
      const empty = document.createElement('div')
      empty.className = 'cleanup-why'
      empty.textContent = 'Nothing recognised above this size and age. Widen the filters.'
      candidatesList.append(empty)
    }
    for (const item of report.candidates) {
      candidatesList.append(cleanupRow(item, () => void toggleCollect(item.node, true)))
    }

    el<HTMLElement>('biggest-title').textContent = 'Biggest items'
    for (const item of report.biggest) {
      biggestList.append(cleanupRow(item, () => void toggleCollect(item.node, true)))
    }

    setStatus(
      `${report.candidates.length} candidate${report.candidates.length === 1 ? '' : 's'} · ${formatBytes(report.total_candidate_bytes)}`,
    )
  } catch (error) {
    showError(`Cleanup report failed: ${String(error)}`)
  }
}

// ─── duplicates ──────────────────────────────────────────────────────────────────────
//
// The words live in duplicates.ts, where they are tested: every group carries the engine's
// caveat as visible text, and every figure that could be read as free space says "up to".
// This half only builds the rows.

function duplicateMemberRow(group: DuplicateGroupModel, index: number): HTMLElement {
  const member = group.members[index]
  if (!member) return document.createElement('div')

  const row = document.createElement('div')
  row.className = member.role === 'keep' ? 'duplicate-member keep' : 'duplicate-member'

  const text = document.createElement('div')
  const name = document.createElement('div')
  name.className = 'cleanup-name'
  name.textContent = member.name
  name.title = member.dir
  const dir = document.createElement('div')
  dir.className = 'cleanup-why'
  dir.textContent = member.dir
  text.append(name, dir)

  const size = document.createElement('span')
  size.className = 'cleanup-size'
  size.textContent = member.bytes

  row.append(text, size)

  if (member.role === 'keep') {
    const badge = document.createElement('span')
    badge.className = 'duplicate-keep'
    badge.textContent = 'keep'
    badge.title = 'The newest copy: this panel does not offer it for deletion.'
    row.append(badge)
    return row
  }

  const add = document.createElement('button')
  add.type = 'button'
  add.className = 'cleanup-add'
  add.textContent = state.collectedIds.includes(member.node) ? '✓' : '+'
  add.title = 'Add this copy to the collector'
  add.addEventListener('click', () => {
    void toggleCollect(member.node, true).then((ok) => {
      // Only claim the copy is collected once the backend said so.
      if (ok) add.textContent = '✓'
    })
  })
  row.append(add)
  return row
}

function duplicateGroupBlock(group: DuplicateGroupModel): HTMLElement {
  const block = document.createElement('div')
  block.className = 'duplicate-group'

  const head = document.createElement('div')
  head.className = 'duplicate-head'
  const title = document.createElement('strong')
  title.textContent = group.title
  const wasted = document.createElement('span')
  wasted.textContent = group.wasted
  head.append(title, wasted)

  const members = document.createElement('div')
  members.className = 'duplicate-members'
  for (let i = 0; i < group.members.length; i += 1) {
    members.append(duplicateMemberRow(group, i))
  }

  // Visible text, never a tooltip: a group read without this line is a space promise the
  // disk may not keep.
  const caveat = document.createElement('p')
  caveat.className = 'duplicate-caveat'
  caveat.textContent = group.caveat

  block.append(head, members, caveat)
  return block
}

async function refreshDuplicates(): Promise<void> {
  const frame = state.stack[state.stack.length - 1]
  if (!state.scanId || !frame) {
    setStatus('Scan something first')
    return
  }

  const floor = Number(el<HTMLSelectElement>('duplicates-floor').value)
  duplicatesList.replaceChildren()
  duplicatesHeadline.textContent = 'Comparing files…'
  duplicatesNote.hidden = true
  setStatus('Comparing files by size, then by content…')

  try {
    const report = await bridge.duplicates(state.scanId, floor)
    const model = duplicatesModel(report)

    duplicatesHeadline.textContent = model.headline
    duplicatesNote.hidden = model.note === null
    duplicatesNote.textContent = model.note ?? ''

    if (model.groups.length === 0) {
      const empty = document.createElement('div')
      empty.className = 'cleanup-why'
      empty.textContent = 'Nothing to show. Lower the floor, or scan a folder with more files.'
      duplicatesList.append(empty)
    }
    for (const group of model.groups) {
      duplicatesList.append(duplicateGroupBlock(group))
    }

    setStatus(
      `${report.groups.length} duplicate group${report.groups.length === 1 ? '' : 's'} · ${(report.duration_ms / 1000).toFixed(1)}s`,
    )
  } catch (error) {
    showError(`Duplicate search failed: ${String(error)}`)
  }
}

/** Type a path and scan it: faster than any dialog when you already know the answer. */
async function scanTypedPath(): Promise<void> {
  const input = el<HTMLInputElement>('manual-path')
  const raw = input.value.trim()
  if (raw.length === 0) {
    setStatus('Type a path to scan, for example /tmp')
    return
  }

  // Only `~` is expanded. Environment variables are deliberately not: a path is data, and
  // the shell's expansion rules are not ours to guess.
  const expanded = raw === '~' ? homePath : raw.startsWith('~/') ? `${homePath}/${raw.slice(2)}` : raw

  overview.hidden = true
  await scanPath(expanded)

  // A bad path leaves the user on an empty stage; bring the overview back with the text intact.
  if (status.textContent?.startsWith('Scan failed')) {
    overview.hidden = false
    input.focus()
    input.select()
  }
}

async function pickFolder(): Promise<void> {
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const chosen = await open({ directory: true, multiple: false, title: 'Choose a folder to scan' })
    if (typeof chosen === 'string' && chosen.length > 0) {
      overview.hidden = true
      await scanPath(chosen)
    }
  } catch (error) {
    showError(`Could not open the folder picker: ${String(error)}`)
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
