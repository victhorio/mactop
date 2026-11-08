import CMactop
import Foundation

public enum UpdateFrequency: UInt32, CaseIterable, Sendable {
  case fifth = 5000
  case half = 2000
  case one = 1000
  case two = 500
  case four = 250

  public var interval: Duration { .milliseconds(Int64(rawValue)) }

  public var label: String {
    switch self {
    case .fifth: "0.2 Hz"
    case .half: "0.5 Hz"
    case .one: "1 Hz"
    case .two: "2 Hz"
    case .four: "4 Hz"
    }
  }
}

/// A single consumer owns this instance. Initialization and destruction can block;
/// the app keeps its entire lifetime on a background task. Native measurement
/// handles remain on the joined Rust worker, not the Swift executor.
public final class NativeSampler {
  private var handle: OpaquePointer?

  public init(demo: Bool = false) throws {
    var error = [CChar](repeating: 0, count: 1024)
    guard let handle = mactop_start(demo ? 1 : 0, &error, error.count) else {
      throw CollectionError(
        String(decoding: error.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self))
    }
    self.handle = handle
  }

  deinit { close() }

  public func close() {
    if let handle {
      mactop_destroy(handle)
      self.handle = nil
    }
  }

  public func poll() throws -> MetricsUpdate? {
    guard let handle else { throw CollectionError("The sampler has stopped.") }
    var raw = MactopSnapshot()
    switch mactop_poll(handle, &raw) {
    case 0: return nil
    case 1: return try Self.copy(raw)
    default: throw CollectionError(String(cString: mactop_last_error(handle)))
    }
  }

  public func setFrequency(_ frequency: UpdateFrequency) throws {
    guard let handle else { throw CollectionError("The sampler has stopped.") }
    guard mactop_set_interval(handle, frequency.rawValue) == 0 else {
      throw CollectionError(String(cString: mactop_last_error(handle)))
    }
  }

  private static func copy(_ raw: MactopSnapshot) throws -> MetricsUpdate {
    let tiers = try UnsafeBufferPointer(start: raw.tiers, count: raw.tier_count).map { tier in
      guard let kind = CpuTierKind(rawValue: tier.kind) else {
        throw CollectionError("Unknown CPU tier from the collector.")
      }
      return CpuTier(
        kind: kind,
        cores: Array(UnsafeBufferPointer(start: tier.cores, count: tier.core_count)),
        frequencyGHz: tier.frequency_ghz.isFinite ? tier.frequency_ghz : nil,
        maxFrequencyGHz: tier.max_frequency_ghz.isFinite ? tier.max_frequency_ghz : nil
      )
    }
    guard let pressure = MemoryPressure(rawValue: raw.memory.pressure) else {
      throw CollectionError("Unknown memory pressure from the collector.")
    }
    let memory = Memory(
      app: raw.memory.app_bytes,
      wired: raw.memory.wired_bytes,
      compressed: raw.memory.compressed_bytes,
      cached: raw.memory.cached_bytes,
      free: raw.memory.free_bytes,
      swapUsed: raw.memory.swap_used_bytes,
      swapAllocated: raw.memory.swap_allocated_bytes,
      pressure: pressure,
      pressurePercent: Double(raw.memory.pressure_percent),
      swapInBytesPerSecond: raw.memory.swap_in_bytes_per_sec,
      swapOutBytesPerSecond: raw.memory.swap_out_bytes_per_sec
    )
    let processes = UnsafeBufferPointer(start: raw.processes, count: raw.process_count).map {
      ProcessReading(
        pid: $0.pid, name: string($0.name), cpuPercent: $0.cpu_percent,
        residentBytes: $0.resident_bytes)
    }
    return MetricsUpdate(
      snapshot: MetricsSnapshot(
        machine: string(raw.machine), tiers: tiers, memory: memory, processes: processes),
      time: raw.time_seconds, sequence: raw.sequence, reset: raw.reset != 0,
      delayed: raw.delayed != 0
    )
  }

  private static func string(_ raw: MactopString) -> String {
    String(decoding: UnsafeBufferPointer(start: raw.data, count: raw.len), as: UTF8.self)
  }
}

public struct CollectionError: LocalizedError, Sendable {
  public let message: String
  public var errorDescription: String? { message }

  public init(_ message: String) { self.message = message }
}
