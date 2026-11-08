import MetricsCore
import Testing

@testable import MactopApp

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
