import AppKit
import Combine
import Foundation
import Testing

@testable import MactopApp
@testable import MetricsCore

@Test func deliveryWaitsRespectRateChangesAndBoundEmptyRetries() {
  let start = ContinuousClock.now
  var schedule = DeliverySchedule(at: start)
  #expect(
    schedule.advance(at: start, frequency: .one, rescheduled: false, empty: false)
      == start + .seconds(1))

  let changedAt = start + .milliseconds(200)
  let due = changedAt + .seconds(5)
  #expect(
    schedule.advance(at: changedAt, frequency: .fifth, rescheduled: true, empty: true) == due,
    "A slower rate must wait for the new collection deadline, not poll every 50 ms")
  let retry = due + .milliseconds(50)
  #expect(schedule.advance(at: due, frequency: .fifth, rescheduled: false, empty: true) == retry)
  #expect(
    schedule.advance(at: retry, frequency: .fifth, rescheduled: false, empty: true)
      == retry + .seconds(5), "A stalled collector must not keep fast retries running")

  let resumed = start + .seconds(100)
  #expect(
    schedule.advance(at: resumed, frequency: .four, rescheduled: true, empty: false)
      == resumed + .milliseconds(250), "A rate change must discard the old deadline")
  #expect(
    schedule.advance(
      at: resumed + .milliseconds(250), frequency: .four, rescheduled: false, empty: false)
      == resumed + .milliseconds(500))
}

@Test @MainActor func readingsPublishAtomicallyAndRepeatedFailuresDoNotPublish() {
  let store = MonitorStore(demo: true)
  var states: [MonitorStore.State] = []
  var notifications = 0
  let changes = store.objectWillChange.sink { notifications += 1 }
  let readings = store.$state.dropFirst().sink { states.append($0) }
  for (sequence, time) in [0.0, 1, 62].enumerated() {
    store.accept(historyUpdate(time: time, sequence: UInt64(sequence), reset: sequence == 0))
  }
  #expect(notifications == 3, "Reset and history pruning must not emit extra notifications")
  #expect(states.count == 3)
  #expect(states.allSatisfy { $0.reading?.sequence == $0.cpuHistory.last?.id })
  #expect(states.allSatisfy { $0.reading?.time == $0.cpuHistory.last?.time })
  #expect(store.cpuHistory.map(\.time) == [1, 62])

  store.fail("Interrupted")
  store.fail("Interrupted")
  #expect(states.count == 4)
  #expect(states.last?.isStale == true)
  #expect(states.last?.error == "Interrupted")
  store.setFrequency(.four)
  #expect(states.count == 4, "Frequency changes do not change the measurement state")
  store.accept(historyUpdate(time: 63, sequence: 3))
  #expect(states.count == 5)
  #expect(states.last?.error == nil)
  #expect(states.last?.isStale == false)
  withExtendedLifetime((changes, readings)) {}
}

@Test @MainActor func statusItemsUpdateOnlyWhenTheirDisplayedValuesChange() throws {
  _ = NSApplication.shared
  let store = MonitorStore(demo: true)
  let cpu = MetricStatusItem(panel: .cpu, store: store)
  let memory = MetricStatusItem(panel: .memory, store: store)
  defer {
    NSStatusBar.system.removeStatusItem(cpu.statusItem)
    NSStatusBar.system.removeStatusItem(memory.statusItem)
  }
  #expect(cpu.statusItem.button?.toolTip?.contains("unavailable") == true)
  store.accept(historyUpdate(time: 0, sequence: 0, active: 20.1, reset: true))
  let cpuImage = try #require(cpu.statusItem.button?.image)
  let memoryImage = try #require(memory.statusItem.button?.image)
  #expect(cpu.statusItem.button?.toolTip?.contains("20%") == true)
  #expect(memory.statusItem.button?.toolTip?.contains("40%") == true)

  store.accept(historyUpdate(time: 1, sequence: 1, active: 20.4))
  store.setFrequency(.four)
  #expect(cpu.statusItem.button?.image === cpuImage)
  #expect(memory.statusItem.button?.image === memoryImage)
  store.accept(historyUpdate(time: 2, sequence: 2, active: 20.6))
  #expect(cpu.statusItem.button?.image !== cpuImage)
  #expect(cpu.statusItem.button?.toolTip?.contains("21%") == true)
  #expect(memory.statusItem.button?.image === memoryImage)

  store.fail("Interrupted")
  let staleImage = try #require(cpu.statusItem.button?.image)
  #expect(cpu.statusItem.button?.toolTip?.contains("last reading") == true)
  #expect(memory.statusItem.button?.image !== memoryImage)
  store.fail("Interrupted")
  #expect(cpu.statusItem.button?.image === staleImage)
  store.accept(historyUpdate(time: 3, sequence: 3, active: 20.6))
  #expect(cpu.statusItem.button?.image !== staleImage)
  #expect(cpu.statusItem.button?.toolTip?.contains("last reading") == false)
}

@Test @MainActor func errorsRetainTheLastGoodReadingAndRecoveryClearsStaleness() throws {
  let store = MonitorStore(demo: true)
  #expect(store.reading == nil)
  store.fail("Startup failed")
  #expect(store.reading == nil)
  #expect(store.isStale)

  let sampler = try NativeSampler(demo: true)
  defer { sampler.close() }
  let reading = try #require(try sampler.poll())
  store.accept(reading)
  #expect(store.error == nil)
  #expect(!store.isStale)

  store.fail("Collection failed")
  #expect(store.error == "Collection failed")
  #expect(store.isStale)
  #expect(store.reading?.snapshot.machine == "Apple Silicon")
  #expect(store.reading?.snapshot.processes.count == 20)
  #expect(store.reading?.time == 0)

  store.accept(reading)
  #expect(store.error == nil)
  #expect(!store.isStale)
}

@Test @MainActor func storeStopsAndCanStartANewSampler() async throws {
  let store = MonitorStore(demo: true)
  store.start()
  let deadline = ContinuousClock.now + .seconds(5)
  while store.reading == nil && .now < deadline {
    try await Task.sleep(for: .milliseconds(20))
  }
  #expect(store.reading != nil)
  await store.stop()
  let sequence = store.reading?.sequence
  try await Task.sleep(for: .milliseconds(1100))
  #expect(store.reading?.sequence == sequence)

  store.fail("Stopped")
  store.start()
  let recoveryDeadline = ContinuousClock.now + .seconds(5)
  while store.error != nil && .now < recoveryDeadline {
    try await Task.sleep(for: .milliseconds(20))
  }
  #expect(store.error == nil)
  #expect(!store.isStale)
  await store.stop()
}

@Test @MainActor func frequencyChangesRescheduleCollectionAndInterruptTheOldWait() async throws {
  let store = MonitorStore(demo: true)
  store.setFrequency(.fifth)
  store.start()
  let initialDeadline = ContinuousClock.now + .seconds(5)
  while store.reading == nil && .now < initialDeadline {
    try await Task.sleep(for: .milliseconds(20))
  }
  let initialSequence = store.reading?.sequence
  #expect(initialSequence == 0)
  try await Task.sleep(for: .milliseconds(1300))
  #expect(
    store.reading?.sequence == initialSequence, "0.2 Hz must slow collection, not just the UI")

  store.setFrequency(.four)
  let fastDeadline = ContinuousClock.now + .seconds(3)
  while (store.reading?.sequence ?? 0) < 4 && .now < fastDeadline {
    try await Task.sleep(for: .milliseconds(20))
  }
  #expect((store.reading?.sequence ?? 0) >= 4, "A faster rate must interrupt the five-second wait")
  #expect(store.reading?.reset == false, "Rate changes must not restart the sampler")
  #expect(store.error == nil)
  #expect(!store.isStale)

  store.setFrequency(.fifth)
  try await Task.sleep(for: .milliseconds(200))
  let slowSequence = store.reading?.sequence
  try await Task.sleep(for: .milliseconds(1200))
  #expect(store.reading?.sequence == slowSequence)
  let stoppingAt = ContinuousClock.now
  await store.stop()
  #expect(stoppingAt.duration(to: .now) < .seconds(1), "Quit must interrupt a slow update wait")
}

@Test @MainActor func cpuHistoryUsesSampleTimeAndStaysBoundedAcrossFrequencyChanges() {
  let store = MonitorStore(demo: true)
  store.setFrequency(.four)
  let times = [0, 0.25, 2, 14.5, 15.25, 16.25, 40.25, 70, 74.75, 75]
  for (sequence, time) in times.enumerated() {
    if sequence == 5 { store.setFrequency(.fifth) }
    store.accept(
      historyUpdate(time: time, sequence: UInt64(sequence), active: time == 15.25 ? 89 : 20))
  }
  #expect(store.cpuHistory.map(\.time) == [14.5, 15.25, 16.25, 40.25, 70, 74.75, 75])
  #expect(store.cpuHistory[1].percent == 89, "Retain earlier peaks, not just the latest value")

  store.accept(historyUpdate(time: 75.25, sequence: 10))
  #expect(store.cpuHistory.first?.time == 15.25, "Retain only one left-edge predecessor")
  store.setFrequency(.four)
  for index in 1...1200 {
    store.accept(historyUpdate(time: 75.25 + Double(index) / 4, sequence: UInt64(10 + index)))
  }
  #expect(store.cpuHistory.count == 241)
  #expect(store.cpuHistory.first?.time == 315.25)
  #expect(store.cpuHistory.last?.time == 375.25)

  store.accept(historyUpdate(time: 0, sequence: 0, active: 33, reset: true))
  #expect(store.cpuHistory.count == 1)
  #expect(store.cpuHistory.first?.time == 0)
  #expect(store.cpuHistory.first?.percent == 33)
}

@Test @MainActor func cpuHistoryBreaksAtErrorsMissingReadingsAndDelayedSamples() {
  let store = MonitorStore(demo: true)
  store.accept(historyUpdate(time: 0, sequence: 0, reset: true))
  store.accept(historyUpdate(time: 1, sequence: 1))
  store.fail("Interrupted")
  #expect(store.cpuHistory.map(\.time) == [0, 1], "Errors retain the last good history")
  store.accept(historyUpdate(time: 3, sequence: 2))
  store.accept(historyUpdate(time: 4, sequence: 3))
  store.accept(historyUpdate(time: 6, sequence: 5))
  store.accept(historyUpdate(time: 8, sequence: 6, delayed: true))
  store.accept(historyUpdate(time: 9, sequence: 7))
  let segments = store.cpuHistory.map(\.segment)
  #expect(segments[0] == segments[1])
  #expect(segments[1] != segments[2], "Do not connect across a collection error")
  #expect(segments[2] == segments[3])
  #expect(segments[3] != segments[4], "Do not connect across an undelivered sample")
  #expect(segments[4] != segments[5], "Do not connect across a delayed measurement")
  #expect(segments[5] == segments[6])
}

@Test @MainActor func consumerWorkDoesNotAddToTheSelectedUpdateInterval() async throws {
  let store = MonitorStore(demo: true)
  var sequences: [UInt64] = []
  let observation = store.$state.compactMap(\.reading).sink { reading in
    sequences.append(reading.sequence)
    // Model view updating that fits within the 250 ms sampling interval.
    Thread.sleep(forTimeInterval: 0.12)
  }
  store.setFrequency(.four)
  store.start()
  let deadline = ContinuousClock.now + .seconds(4)
  while (sequences.last ?? 0) < 8 && .now < deadline {
    try await Task.sleep(for: .milliseconds(10))
  }
  await store.stop()
  withExtendedLifetime(observation) {}
  #expect(sequences == [0, 1, 2, 3, 4, 5, 6, 7, 8])
}

private func historyUpdate(
  time: Double, sequence: UInt64, active: Double = 20, reset: Bool = false, delayed: Bool = false
) -> MetricsUpdate {
  MetricsUpdate(
    snapshot: MetricsSnapshot(
      machine: "History fixture",
      tiers: [
        CpuTier(kind: .performance, cores: [active], frequencyGHz: nil, maxFrequencyGHz: nil)
      ],
      memory: Memory(
        app: 5, wired: 2, compressed: 1, cached: 7, free: 5,
        swapUsed: 0, swapAllocated: 0, pressure: .normal, pressurePercent: 20,
        swapInBytesPerSecond: 0, swapOutBytesPerSecond: 0), processes: []),
    time: time, sequence: sequence, reset: reset, delayed: delayed)
}
