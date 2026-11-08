import Testing

@testable import MetricsCore

@Test func metricDefinitionsUseCoreWeightingAndExcludeReclaimableCache() {
  let memory = Memory(
    app: 5, wired: 2, compressed: 1, cached: 7, free: 5,
    swapUsed: 3, swapAllocated: 6, pressure: .critical,
    swapInBytesPerSecond: 4096, swapOutBytesPerSecond: 0)
  let snapshot = MetricsSnapshot(
    machine: "Asymmetric fixture",
    tiers: [
      CpuTier(kind: .efficiency, cores: [10, 30], frequencyGHz: nil, maxFrequencyGHz: 2.8),
      CpuTier(kind: .performance, cores: [90], frequencyGHz: 3.2, maxFrequencyGHz: 4.1),
    ], memory: memory, processes: [])

  #expect(snapshot.coreCount == 3)
  #expect(abs(snapshot.cpuPercent - 43.333333333333336) < 1e-12)
  #expect(snapshot.tiers[0].activePercent == 20)
  #expect(snapshot.tiers[0].frequencyGHz == nil)
  #expect(memory.used == 8)
  #expect(memory.available == 12)
  #expect(memory.total == 20)
  #expect(memory.usedPercent == 40)
  #expect(memory.pressure == .critical)
}

@Test func swiftBridgeOwnsCompleteReadingsAfterSamplerDestruction() throws {
  let sampler = try NativeSampler(demo: true)
  let update = try #require(try sampler.poll())
  sampler.close()
  sampler.close()

  #expect(update.time == 0)
  #expect(update.sequence == 0)
  #expect(update.reset)
  #expect(!update.delayed)
  #expect(update.snapshot.machine == "Apple Silicon")
  #expect(update.snapshot.tiers.map(\.kind) == [.efficiency, .performance, .superCore])
  #expect(update.snapshot.tiers.map { $0.cores.count } == [4, 6, 6])
  #expect(update.snapshot.tiers.map(\.maxFrequencyGHz) == [2.8, 4.1, 4.6])
  #expect(update.snapshot.memory.total == 32 * 1_073_741_824)
  #expect(update.snapshot.memory.pressure == .normal)
  #expect(update.snapshot.memory.swapUsed == 1_503_238_553)
  #expect(update.snapshot.memory.swapAllocated == 3 * 1_073_741_824)
  #expect(update.snapshot.memory.swapInBytesPerSecond == 0)
  #expect(update.snapshot.memory.swapOutBytesPerSecond == 0)
  #expect(update.snapshot.processes.count == 20)
  let process = try #require(update.snapshot.processes.first { $0.pid == 8241 })
  #expect(process.name == "OrbStack")
  #expect(abs(process.cpuPercent - 339.26475) < 1e-8)
  #expect(process.residentBytes == 7_301_444_403)
  #expect(throws: CollectionError.self) { try sampler.poll() }
}
