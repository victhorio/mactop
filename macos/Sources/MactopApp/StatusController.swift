import AppKit
import Combine
import MetricsCore
import SwiftUI

@MainActor
final class StatusController {
  let items: [MetricStatusItem]

  init(store: MonitorStore) {
    items = MetricPanel.allCases.reversed().map { MetricStatusItem(panel: $0, store: store) }
  }

  func show(_ panel: MetricPanel) {
    for item in items where item.panel != panel { item.popover.performClose(nil) }
    items.first { $0.panel == panel }?.show()
  }
}

@MainActor
final class MetricStatusItem: NSObject {
  let panel: MetricPanel
  let statusItem = NSStatusBar.system.statusItem(withLength: 36)
  let popover = NSPopover()
  private let store: MonitorStore
  private var observation: AnyCancellable?
  private var displayed: StatusReading?

  private struct StatusReading: Equatable {
    let percentage: Int?
    let stale: Bool
  }

  init(panel: MetricPanel, store: MonitorStore) {
    self.panel = panel
    self.store = store
    super.init()
    statusItem.autosaveName = "mactop.\(panel.rawValue)"
    statusItem.button?.target = self
    statusItem.button?.action = #selector(clicked)
    statusItem.button?.sendAction(on: [.leftMouseUp, .rightMouseUp])
    popover.behavior = .transient
    popover.animates = !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    popover.contentViewController = PanelViewController(store: store, panel: panel)
    observation = store.objectWillChange.sink { [weak self] _ in
      Task { @MainActor in self?.update() }
    }
    update()
  }

  @objc private func clicked() {
    if NSApp.currentEvent?.type == .rightMouseUp {
      openMenu()
    } else if popover.isShown {
      popover.performClose(nil)
    } else {
      show()
    }
  }

  func show() {
    guard let button = statusItem.button else { return }
    NSApp.activate()
    popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
    popover.contentViewController?.view.window?.makeKey()
  }

  private func openMenu() {
    makeMenu().popUp(positioning: nil, at: NSEvent.mouseLocation, in: nil)
  }

  func makeMenu() -> NSMenu {
    let menu = NSMenu()
    let title = NSMenuItem(
      title: store.isDemo ? "mactop · Sample data" : "mactop", action: nil, keyEquivalent: "")
    title.isEnabled = false
    menu.addItem(title)
    menu.addItem(.separator())
    menu.addItem(.sectionHeader(title: "Update frequency"))
    for frequency in UpdateFrequency.allCases {
      let option = NSMenuItem(
        title: frequency.label, action: #selector(selectFrequency(_:)), keyEquivalent: "")
      option.target = self
      option.tag = Int(frequency.rawValue)
      option.state = frequency == store.frequency ? .on : .off
      menu.addItem(option)
    }
    menu.addItem(.separator())
    menu.addItem(
      NSMenuItem(
        title: "Quit mactop", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q"))
    return menu
  }

  @objc private func selectFrequency(_ item: NSMenuItem) {
    guard let frequency = UpdateFrequency(rawValue: UInt32(item.tag)) else { return }
    store.setFrequency(frequency)
  }

  private func update() {
    let value = store.reading.map {
      panel == .cpu ? $0.snapshot.cpuPercent : $0.snapshot.memory.usedPercent
    }
    let reading = StatusReading(percentage: value.map { Int($0.rounded()) }, stale: store.isStale)
    if displayed != reading {
      let label = VStack(spacing: 0) {
        Text(panel.menuLabel).font(.system(size: 9, weight: .medium))
        Text(reading.percentage.map { "\($0)%" } ?? "—")
          .font(.system(size: 11, weight: .medium)).monospacedDigit()
      }
      .foregroundStyle(.black)
      .opacity(reading.stale ? 0.45 : 1)
      .frame(width: 34, height: 22)
      let renderer = ImageRenderer(content: label)
      renderer.scale = 2
      let image = renderer.nsImage
      image?.isTemplate = true
      statusItem.button?.image = image
      displayed = reading
    }
    let metric = panel == .cpu ? "CPU active" : "RAM in use"
    let mode = store.isDemo ? "mactop · Sample data" : "mactop"
    let suffix = reading.stale && reading.percentage != nil ? " (last reading)" : ""
    let description =
      "\(mode) · \(metric): \(reading.percentage.map { "\($0)%" } ?? "unavailable")\(suffix)"
    statusItem.button?.toolTip = description
    statusItem.button?.setAccessibilityLabel(description)
    statusItem.button?.setAccessibilityHelp(
      "Click for details. Right-click for update frequency or quit.")
  }
}

@MainActor
final class PanelViewController: NSViewController {
  private let panel: MetricPanelView

  init(store: MonitorStore, panel: MetricPanel) {
    self.panel = MetricPanelView(store: store, panel: panel)
    super.init(nibName: nil, bundle: nil)
  }

  required init?(coder: NSCoder) { nil }

  override func loadView() {
    let hosting = NSHostingView(rootView: panel)
    hosting.translatesAutoresizingMaskIntoConstraints = false
    let material = NSVisualEffectView()
    material.material = .popover
    material.blendingMode = .withinWindow
    material.state = .active
    material.addSubview(hosting)
    NSLayoutConstraint.activate([
      hosting.leadingAnchor.constraint(equalTo: material.leadingAnchor),
      hosting.trailingAnchor.constraint(equalTo: material.trailingAnchor),
      hosting.topAnchor.constraint(equalTo: material.topAnchor),
      hosting.bottomAnchor.constraint(equalTo: material.bottomAnchor),
    ])
    view = material
    preferredContentSize = material.fittingSize
  }

  override func viewDidLayout() {
    super.viewDidLayout()
    let size = view.fittingSize
    if preferredContentSize != size { preferredContentSize = size }
  }
}
