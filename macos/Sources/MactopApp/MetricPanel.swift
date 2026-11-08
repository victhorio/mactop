import AppKit
import Charts
import MetricsCore
import SwiftUI

enum MetricPanel: String, CaseIterable {
  case cpu, memory

  var menuLabel: String { self == .cpu ? "CPU" : "MEM" }
}

struct MetricPanelView: View {
  @Environment(\.colorScheme) private var colorScheme
  @ObservedObject var store: MonitorStore
  let panel: MetricPanel

  private var blue: Color { color(0x3e78ef, 0x82a6ff) }

  var body: some View {
    VStack(alignment: .leading, spacing: 22) {
      if let error = store.error {
        Label(error, systemImage: "exclamationmark.triangle")
          .foregroundStyle(.orange)
          .fixedSize(horizontal: false, vertical: true)
          .padding(10).frame(maxWidth: .infinity, alignment: .leading)
          .background(.orange.opacity(0.08), in: RoundedRectangle(cornerRadius: 6))
      } else if store.isStale {
        Text("Waiting for a fresh reading…").foregroundStyle(.secondary)
      }

      if let snapshot = store.reading?.snapshot {
        if panel == .cpu { cpu(snapshot) } else { memory(snapshot) }
        VStack(spacing: 11) {
          Divider()
          processes(snapshot)
          if panel == .memory {
            Divider()
            HStack {
              Text("Swap").foregroundStyle(.secondary)
              Spacer()
              amount(bytes(snapshot.memory.swapUsed))
            }
            .help(
              "\(bytes(snapshot.memory.swapUsed)) of \(bytes(snapshot.memory.swapAllocated)) swap used\n"
                + "Swap in: \(rate(snapshot.memory.swapInBytesPerSecond)) · Out: \(rate(snapshot.memory.swapOutBytesPerSecond))"
            )
          }
        }
      } else {
        HStack(spacing: 10) {
          ProgressView().controlSize(.small)
          Text(store.error == nil ? "Measuring…" : "Retrying automatically…")
            .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, minHeight: 70, alignment: .leading)
      }
    }
    .font(.system(size: 12))
    .foregroundStyle(color(0x292825, 0xedeeeb))
    .monospacedDigit()
    .padding(20)
    .frame(width: 312)
    .background(color(0xffffff, 0x292b29))
  }

  private func headline(_ value: Double, label: String, weight: Font.Weight = .medium) -> some View
  {
    HStack(alignment: .firstTextBaseline, spacing: 2) {
      Text(String(format: "%.0f", value)).font(.system(size: 43, weight: weight))
        .tracking(-1.8)
      Text("%").font(.system(size: 21)).foregroundStyle(.secondary)
    }
    .fixedSize()
    .accessibilityElement(children: .ignore)
    .accessibilityLabel("\(label): \(percent(value))")
  }

  private func cpu(_ snapshot: MetricsSnapshot) -> some View {
    let cores = snapshot.tiers.flatMap { tier in
      tier.cores.indices.map { (tier: tier, index: $0) }
    }
    return VStack(spacing: 8) {
      HStack(spacing: 9) {
        headline(snapshot.cpuPercent, label: "CPU activity")
          .frame(width: 86, alignment: .leading)
        cpuHistory
      }
      HStack(alignment: .top, spacing: 9) {
        Text("\(snapshot.coreCount) cores")
          .font(.system(size: 10.5)).foregroundStyle(.secondary)
          .fixedSize().padding(.top, 3)
        LazyVGrid(
          columns: [GridItem(.adaptive(minimum: 10, maximum: 10), spacing: 4)],
          alignment: .trailing, spacing: 4
        ) {
          ForEach(cores.indices, id: \.self) { position in
            let (tier, index) = cores[position]
            RoundedRectangle(cornerRadius: 2.5)
              .fill(tierColor(tier.kind).opacity(0.12))
              .overlay(alignment: .bottom) {
                RoundedRectangle(cornerRadius: 1.5)
                  .fill(tierColor(tier.kind))
                  .frame(height: 20 * tier.cores[index] / 100)
              }
              .clipShape(RoundedRectangle(cornerRadius: 2.5))
              .frame(width: 10, height: 20)
              .help(
                "\(tier.kind.name) core \(index + 1): \(percent(tier.cores[index]))\n"
                  + "Active clock: \(clock(tier.frequencyGHz)). Maximum: \(clock(tier.maxFrequencyGHz))."
              )
              .accessibilityElement(children: .ignore)
              .accessibilityLabel(
                "\(tier.kind.name) core \(index + 1): \(percent(tier.cores[index]))")
          }
        }
        .frame(maxWidth: CGFloat(cores.count) * 14)
        .frame(maxWidth: .infinity, alignment: .trailing)
      }
    }
    .help(
      "\(snapshot.machine)\(store.isDemo ? " · Sample data" : "")\n"
        + "CPU is active residency, weighted by core count. Clocks are active-weighted interval averages. "
        + "Core numbers are positions within each tier, not logical CPU IDs."
    )
  }

  private var cpuHistory: some View {
    let latest = store.cpuHistory.last?.time ?? 0
    return VStack(spacing: 0) {
      Chart {
        ForEach(store.cpuHistory) { point in
          AreaMark(
            x: .value("Time", point.time), yStart: .value("Activity", 0),
            yEnd: .value("Activity", point.percent), series: .value("Segment", point.segment)
          )
          .foregroundStyle(
            .linearGradient(
              colors: [blue.opacity(0.15), blue.opacity(0.015)],
              startPoint: .top, endPoint: .bottom)
          )
          .interpolationMethod(.monotone)
          LineMark(
            x: .value("Time", point.time), y: .value("Activity", point.percent),
            series: .value("Segment", point.segment)
          )
          .foregroundStyle(blue).lineStyle(StrokeStyle(lineWidth: 1.7, lineJoin: .round))
          .interpolationMethod(.monotone)
        }
        if let point = store.cpuHistory.last {
          PointMark(x: .value("Time", point.time), y: .value("Activity", point.percent))
            .foregroundStyle(blue).symbolSize(15)
        }
      }
      .chartXScale(domain: (latest - 60)...latest)
      .chartYScale(domain: 0...100)
      .chartXAxis(.hidden).chartYAxis(.hidden).chartLegend(.hidden)
      .frame(height: 53).clipped()
      .accessibilityLabel("CPU activity over the last 60 seconds, from zero to 100 percent")
      HStack {
        Spacer()
        Text("60 s")
      }
      .font(.system(size: 9)).foregroundStyle(.secondary)
      .frame(height: 11)
    }
  }

  private func memory(_ snapshot: MetricsSnapshot) -> some View {
    let memory = snapshot.memory
    let segments: [(value: UInt64, color: Color)] = [
      (memory.app, color(0x8b67de, 0xb598f5)),
      (memory.wired, color(0xc18c3e, 0xe3b36c)),
      (memory.compressed, color(0xdb6e9b, 0xee94b8)),
      (memory.available, color(0xeeefec, 0x3a3e3b)),
    ]
    return VStack(alignment: .leading, spacing: 0) {
      HStack(alignment: .firstTextBaseline) {
        VStack(alignment: .leading, spacing: 6) {
          headline(memory.usedPercent, label: "Memory used")
          Text("Memory").foregroundStyle(.secondary)
        }
        .help(
          "\(bytes(memory.used)) of \(bytes(memory.total)) in use · \(bytes(memory.available)) available\n"
            + "\(bytes(memory.cached)) cached · \(bytes(memory.free)) free")
        Spacer(minLength: 24)
        VStack(alignment: .trailing, spacing: 6) {
          headline(
            memory.pressurePercent, label: "\(memory.pressure.name) memory pressure",
            weight: .regular
          )
          Text("Pressure").foregroundStyle(.secondary)
        }
        .help(
          "\(memory.pressure.name) memory pressure\n"
            + "100 minus the kernel's available-memory percentage; separate from RAM usage. Lower means less pressure."
        )
      }
      if memory.pressure != .normal {
        Text("\(memory.pressure.name) memory pressure")
          .foregroundStyle(memory.pressure == .critical ? .red : .orange)
          .padding(.top, 8)
      }
      GeometryReader { geometry in
        HStack(spacing: 0) {
          ForEach(segments.indices, id: \.self) { index in
            segments[index].color
              .frame(
                width: geometry.size.width * Double(segments[index].value) / Double(memory.total))
          }
        }
        .clipShape(RoundedRectangle(cornerRadius: 3))
      }
      .frame(height: 7).padding(.top, 18).accessibilityHidden(true)
      HStack(alignment: .top, spacing: 10) {
        ForEach(Array(zip(["App", "Wired", "Compressed"], segments.prefix(3))), id: \.0) {
          label, segment in
          VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 5) {
              RoundedRectangle(cornerRadius: 1).fill(segment.color).frame(width: 4, height: 4)
              Text(label).foregroundStyle(.secondary)
            }
            amount(bytes(segment.value))
          }
          .frame(maxWidth: .infinity, alignment: .leading)
        }
      }
      .padding(.top, 11)
    }
  }

  private func processes(_ snapshot: MetricsSnapshot) -> some View {
    let sorted = snapshot.processes.sorted {
      if panel == .cpu {
        return $0.cpuPercent == $1.cpuPercent ? $0.pid < $1.pid : $0.cpuPercent > $1.cpuPercent
      }
      return $0.residentBytes == $1.residentBytes
        ? $0.pid < $1.pid : $0.residentBytes > $1.residentBytes
    }
    return VStack(spacing: 0) {
      if sorted.isEmpty {
        Text("No accessible processes").foregroundStyle(.secondary)
          .frame(maxWidth: .infinity, alignment: .leading)
      }
      ForEach(sorted.prefix(4)) { process in
        ProcessRow(
          process: process,
          value: amount(panel == .cpu ? percent(process.cpuPercent) : bytes(process.residentBytes))
            .font(.system(size: panel == .cpu ? 11.5 : 12)),
          showIcon: !store.isDemo)
      }
    }
    .help(
      panel == .cpu
        ? "100% is one logical core; processes can exceed 100%. Only processes accessible to this user appear."
        : "Resident memory can include shared pages. Process sizes do not add up to system app memory."
    )
  }

  private func tierColor(_ kind: CpuTierKind) -> Color {
    switch kind {
    case .efficiency: color(0x269b72, 0x56c89f)
    case .performance: blue
    case .superCore: color(0xdc568b, 0xf58aba)
    }
  }

  private func color(_ light: UInt32, _ dark: UInt32) -> Color {
    let hex = colorScheme == .light ? light : dark
    return Color(
      red: Double((hex >> 16) & 0xff) / 255,
      green: Double((hex >> 8) & 0xff) / 255,
      blue: Double(hex & 0xff) / 255)
  }

  private func clock(_ value: Double?) -> String {
    value.map { String(format: "%.2f GHz", $0) } ?? "unavailable"
  }

  private func amount(_ value: String) -> Text {
    let number = value.prefix { $0 != " " && $0 != "%" }
    return Text(String(number))
      + Text(String(value.dropFirst(number.count))).foregroundColor(.secondary)
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

// Disambiguate the property wrapper from the macOS 27 SDK's State macro.
private typealias ViewState<Value> = SwiftUI.State<Value>

private struct ProcessRow: View {
  let process: ProcessReading
  let value: Text
  let showIcon: Bool
  @ViewState private var icon: NSImage?

  var body: some View {
    HStack(spacing: 9) {
      Group {
        if let icon {
          Image(nsImage: icon).resizable()
        } else {
          Image(systemName: "terminal").foregroundStyle(.secondary)
        }
      }
      .frame(width: 17, height: 17).accessibilityHidden(true)
      Text(process.name).lineLimit(1).truncationMode(.middle)
      Spacer(minLength: 4)
      value.fixedSize()
    }
    .frame(height: 29)
    .help("PID \(process.pid) · \(process.name)")
    .accessibilityElement(children: .combine)
    .task(id: process.name) {
      if showIcon {
        icon = NSRunningApplication(processIdentifier: pid_t(process.pid))?.icon
      }
    }
  }
}
