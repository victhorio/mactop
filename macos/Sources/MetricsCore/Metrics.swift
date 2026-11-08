import Foundation

public enum CpuTierKind: UInt32, Sendable {
  case efficiency, performance, superCore

  public var name: String {
    switch self {
    case .efficiency: "Efficiency"
    case .performance: "Performance"
    case .superCore: "Super"
    }
  }
}

public struct CpuTier: Sendable, Identifiable {
  public let kind: CpuTierKind
  public let cores: [Double]
  public let frequencyGHz: Double?
  public let maxFrequencyGHz: Double?

  public var id: CpuTierKind { kind }
  public var activePercent: Double { cores.reduce(0, +) / Double(cores.count) }
}

public enum MemoryPressure: UInt32, Sendable {
  case normal, elevated, critical

  public var name: String {
    switch self {
    case .normal: "Normal"
    case .elevated: "Elevated"
    case .critical: "Critical"
    }
  }
}

public struct Memory: Sendable {
  public let app: UInt64
  public let wired: UInt64
  public let compressed: UInt64
  public let cached: UInt64
  public let free: UInt64
  public let swapUsed: UInt64
  public let swapAllocated: UInt64
  public let pressure: MemoryPressure
  public let swapInBytesPerSecond: Double
  public let swapOutBytesPerSecond: Double

  public var used: UInt64 { app + wired + compressed }
  public var available: UInt64 { cached + free }
  public var total: UInt64 { used + available }
  public var usedPercent: Double { Double(used) / Double(total) * 100 }
}

public struct ProcessReading: Sendable, Identifiable {
  public let pid: UInt32
  public let name: String
  public let cpuPercent: Double
  public let residentBytes: UInt64

  public var id: UInt32 { pid }
}

public struct MetricsSnapshot: Sendable {
  public let machine: String
  public let tiers: [CpuTier]
  public let memory: Memory
  public let processes: [ProcessReading]

  public var coreCount: Int { tiers.reduce(0) { $0 + $1.cores.count } }
  public var cpuPercent: Double {
    tiers.reduce(0) { $0 + $1.cores.reduce(0, +) } / Double(coreCount)
  }
}

public struct MetricsUpdate: Sendable {
  public let snapshot: MetricsSnapshot
  public let time: Double
  public let sequence: UInt64
  public let reset: Bool
  public let delayed: Bool
}
