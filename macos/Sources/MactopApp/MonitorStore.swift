import Combine
import Foundation
import MetricsCore

struct CPUHistoryPoint: Identifiable {
  let id: UInt64
  let time: Double
  let percent: Double
  let segment: UInt64
}

struct DeliverySchedule {
  private var deadline: ContinuousClock.Instant
  private var retried = false

  init(at now: ContinuousClock.Instant = .now) { deadline = now }

  mutating func advance(
    at now: ContinuousClock.Instant, frequency: UpdateFrequency, rescheduled: Bool, empty: Bool
  ) -> ContinuousClock.Instant {
    if rescheduled || now > deadline + frequency.interval { deadline = now }
    if empty && !rescheduled && !retried {
      deadline = now + .milliseconds(50)
      retried = true
    } else {
      deadline += frequency.interval
      retried = false
    }
    return deadline
  }
}

@MainActor
final class MonitorStore: ObservableObject {
  struct State {
    var reading: MetricsUpdate?
    var cpuHistory: [CPUHistoryPoint] = []
    var error: String?
    var isStale = false
  }

  @Published private(set) var state = State()
  @Published private(set) var frequency: UpdateFrequency = .one
  let isDemo: Bool

  var reading: MetricsUpdate? { state.reading }
  var cpuHistory: [CPUHistoryPoint] { state.cpuHistory }
  var error: String? { state.error }
  var isStale: Bool { state.isStale }

  private var worker: Task<Void, Never>?
  private var delay: Task<Void, Never>?
  private var lastReceived: ContinuousClock.Instant?

  init(demo: Bool = false) { isDemo = demo }

  deinit { worker?.cancel() }

  func start() {
    guard worker == nil else { return }
    let demo = isDemo
    worker = Task.detached(priority: .utility) { [weak self] in
      while !Task.isCancelled {
        do {
          let sampler = try NativeSampler(demo: demo)
          defer { sampler.close() }
          var appliedFrequency = UpdateFrequency.one
          var schedule = DeliverySchedule()
          while !Task.isCancelled {
            guard let frequency = await self?.frequency else { return }
            let polledAt = ContinuousClock.now
            let rescheduled = frequency != appliedFrequency
            var empty = false
            do {
              if rescheduled {
                try sampler.setFrequency(frequency)
                appliedFrequency = frequency
              }
              let reading = try sampler.poll()
              if Task.isCancelled { break }
              if let reading { await self?.accept(reading) } else { await self?.checkStaleness() }
              empty = reading == nil
            } catch {
              await self?.fail(error.localizedDescription)
            }
            // Keep timer coalescing and view work from accumulating across refreshes.
            let nextPoll = schedule.advance(
              at: polledAt, frequency: frequency, rescheduled: rescheduled, empty: empty)
            await self?.wait(at: frequency, until: nextPoll)
          }
        } catch {
          await self?.fail(error.localizedDescription)
          do { try await Task.sleep(for: .seconds(5)) } catch { return }
        }
      }
    }
  }

  func stop() async {
    guard let current = worker else { return }
    current.cancel()
    await current.value
    if worker == current { worker = nil }
  }

  func setFrequency(_ frequency: UpdateFrequency) {
    guard self.frequency != frequency else { return }
    self.frequency = frequency
    delay?.cancel()
  }

  private func wait(at frequency: UpdateFrequency, until deadline: ContinuousClock.Instant) async {
    guard self.frequency == frequency, !Task.isCancelled else { return }
    let current = Task<Void, Never> { try? await Task.sleep(until: deadline, clock: .continuous) }
    delay = current
    await withTaskCancellationHandler {
      await current.value
    } onCancel: {
      current.cancel()
    }
    if delay == current { delay = nil }
  }

  func accept(_ reading: MetricsUpdate) {
    var next = state
    if reading.reset { next.cpuHistory.removeAll(keepingCapacity: true) }
    let previous = next.cpuHistory.last
    let continuous =
      previous.map { reading.sequence == $0.id + 1 } == true
      && !reading.delayed && !isStale
    let segment = (previous?.segment ?? 0) + (continuous ? 0 : 1)
    next.cpuHistory.append(
      CPUHistoryPoint(
        id: reading.sequence, time: reading.time, percent: reading.snapshot.cpuPercent,
        segment: segment))
    // Keep one predecessor so the chart can clip a line crossing its left edge.
    while next.cpuHistory.count > 1 && next.cpuHistory[1].time <= reading.time - 60 {
      next.cpuHistory.removeFirst()
    }
    next.reading = reading
    next.error = nil
    next.isStale = false
    lastReceived = .now
    state = next
  }

  func fail(_ message: String) {
    guard error != message || !isStale else { return }
    var next = state
    next.error = message
    next.isStale = true
    state = next
  }

  private func checkStaleness() {
    if !isStale, let lastReceived,
      lastReceived.duration(to: .now) > max(.seconds(3), frequency.interval * 3)
    {
      state.isStale = true
    }
  }
}
