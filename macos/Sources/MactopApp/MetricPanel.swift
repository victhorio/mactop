import MetricsCore
import SwiftUI

enum MetricPanel: String, CaseIterable {
  case cpu, memory

  var title: String { self == .cpu ? "CPU" : "Memory" }
  var menuLabel: String { self == .cpu ? "CPU" : "MEM" }
}

struct MetricPanelView: View {
  @ObservedObject var store: MonitorStore
  let panel: MetricPanel

  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      HStack {
        Text(panel.title).font(.headline)
        Spacer()
        Group {
          if store.reading == nil {
            Text(store.error == nil ? "Starting…" : "Unavailable")
          } else {
            Text(store.isStale ? "Last reading" : store.isDemo ? "Sample data" : "Live")
          }
        }
        .font(.caption).foregroundStyle(store.isStale ? .orange : .secondary)
      }

      if let error = store.error {
        Label(error, systemImage: "exclamationmark.triangle")
          .font(.caption).foregroundStyle(.orange).fixedSize(horizontal: false, vertical: true)
      } else if store.isStale {
        Text("Waiting for a fresh reading…").font(.caption).foregroundStyle(.secondary)
      }

      if let snapshot = store.reading?.snapshot {
        Text(snapshot.machine).font(.caption).foregroundStyle(.secondary)
        if panel == .cpu { cpu(snapshot) } else { memory(snapshot) }
        processes(snapshot)
      } else {
        HStack(spacing: 10) {
          ProgressView().controlSize(.small)
          Text(store.error == nil ? "Measuring…" : "Retrying automatically…")
            .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, minHeight: 70, alignment: .leading)
      }

      HStack {
        Text("\(store.isDemo ? "Synthetic" : "Native") readings · \(store.frequency.label)")
          .font(.caption2).foregroundStyle(.secondary)
        Spacer()
        Button("Quit") { MactopApplication.quit() }.controlSize(.small)
      }
    }
    .monospacedDigit()
    .padding(16)
    .frame(width: 368)
  }

  private func cpu(_ snapshot: MetricsSnapshot) -> some View {
    VStack(alignment: .leading, spacing: 12) {
      HStack(alignment: .firstTextBaseline) {
        Text(percent(snapshot.cpuPercent)).font(.system(size: 28, weight: .medium))
        Text("active · \(snapshot.coreCount) cores").foregroundStyle(.secondary)
      }
      ProgressView(value: snapshot.cpuPercent, total: 100).accessibilityLabel("CPU activity")
      ForEach(snapshot.tiers) { tier in
        GroupBox {
          VStack(alignment: .leading, spacing: 7) {
            HStack {
              Text("\(tier.cores.count) cores").foregroundStyle(.secondary)
              Spacer()
              Text(percent(tier.activePercent))
            }
            HStack {
              Text("Active clock").foregroundStyle(.secondary)
              Spacer()
              Text(tier.frequencyGHz.map { String(format: "%.2f GHz", $0) } ?? "—")
            }
            if let maximum = tier.maxFrequencyGHz {
              HStack {
                Text("Maximum").foregroundStyle(.secondary)
                Spacer()
                Text(String(format: "%.2f GHz", maximum))
              }
            }
            Text(
              tier.cores.enumerated().map { "\($0.offset + 1): \(percent($0.element))" }
                .joined(separator: "   ")
            )
            .font(.caption2).foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
          }
          .font(.callout).padding(4)
        } label: {
          Text(tier.kind.name)
        }
      }
    }
    .help(
      "CPU is active residency, weighted by core count. Active clock is an interval average while busy. Core numbers are positions within each tier."
    )
  }

  private func memory(_ snapshot: MetricsSnapshot) -> some View {
    let memory = snapshot.memory
    return VStack(alignment: .leading, spacing: 12) {
      HStack(alignment: .firstTextBaseline) {
        Text(percent(memory.usedPercent)).font(.system(size: 28, weight: .medium))
        Text("in use").foregroundStyle(.secondary)
        Spacer()
        Text("\(bytes(memory.used)) / \(bytes(memory.total))").font(.callout)
      }
      ProgressView(value: memory.usedPercent, total: 100).accessibilityLabel("RAM in use")
      GroupBox("RAM") {
        VStack(spacing: 6) {
          row("App", bytes(memory.app))
          row("Wired", bytes(memory.wired))
          row("Compressed", bytes(memory.compressed))
          Divider()
          row("Cached", bytes(memory.cached))
          row("Free", bytes(memory.free))
          row("Available", bytes(memory.available))
          Divider()
          row("Pressure", memory.pressure.name)
        }
        .padding(4)
      }
      GroupBox("Swap") {
        VStack(spacing: 6) {
          row("Used", "\(bytes(memory.swapUsed)) / \(bytes(memory.swapAllocated))")
          row("In", rate(memory.swapInBytesPerSecond))
          row("Out", rate(memory.swapOutBytesPerSecond))
        }
        .padding(4)
      }
    }
    .help(
      "In use is app + wired + physically compressed memory. Available is cached + free. Pressure is a separate macOS state. Sizes use binary units."
    )
  }

  private func processes(_ snapshot: MetricsSnapshot) -> some View {
    let sorted = snapshot.processes.sorted {
      if panel == .cpu {
        return $0.cpuPercent == $1.cpuPercent ? $0.pid < $1.pid : $0.cpuPercent > $1.cpuPercent
      }
      return $0.residentBytes == $1.residentBytes
        ? $0.pid < $1.pid : $0.residentBytes > $1.residentBytes
    }
    return GroupBox(panel == .cpu ? "Top processes · CPU" : "Top processes · RAM") {
      VStack(spacing: 7) {
        if sorted.isEmpty {
          Text("No accessible processes").foregroundStyle(.secondary)
        }
        ForEach(sorted.prefix(5)) { process in
          HStack(spacing: 12) {
            Text(process.name).lineLimit(1).truncationMode(.middle)
              .help("PID \(process.pid) · \(process.name)")
            Spacer(minLength: 0)
            Text(panel == .cpu ? percent(process.cpuPercent) : bytes(process.residentBytes))
              .foregroundStyle(.secondary).fixedSize()
          }
        }
      }
      .font(.callout).padding(4)
    }
    .help(
      panel == .cpu
        ? "100% is one logical core; processes can exceed 100%. Only processes accessible to this user appear."
        : "Resident memory can include shared pages. Process sizes do not add up to system app memory."
    )
  }

  private func row(_ label: String, _ value: String) -> some View {
    HStack {
      Text(label).foregroundStyle(.secondary)
      Spacer()
      Text(value)
    }
    .font(.callout)
  }

  private func percent(_ value: Double) -> String { String(format: "%.0f%%", value) }

  private func bytes(_ value: UInt64) -> String {
    let gib = Double(value) / 1_073_741_824
    return gib >= 1
      ? String(format: "%.1f GiB", gib) : String(format: "%.0f MiB", Double(value) / 1_048_576)
  }

  private func rate(_ value: Double) -> String {
    if value == 0 { return "0 B/s" }
    if value < 1024 { return String(format: "%.0f B/s", value) }
    if value < 1_048_576 { return String(format: "%.1f KiB/s", value / 1024) }
    return String(format: "%.1f MiB/s", value / 1_048_576)
  }
}
