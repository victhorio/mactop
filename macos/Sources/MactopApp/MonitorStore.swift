import Combine
import Foundation
import MetricsCore

struct CPUHistoryPoint: Identifiable {
  let id: UInt64
  let time: Double
  let percent: Double
  let segment: UInt64
}

@MainActor
final class MonitorStore: ObservableObject {
  @Published private(set) var reading: MetricsUpdate?
  @Published private(set) var cpuHistory: [CPUHistoryPoint] = []
  @Published private(set) var error: String?
  @Published private(set) var isStale = false
  @Published private(set) var frequency: UpdateFrequency = .one
  let isDemo: Bool

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
          var nextPoll = ContinuousClock.now
          while !Task.isCancelled {
            guard let frequency = await self?.frequency else { return }
            let polledAt = ContinuousClock.now
            if polledAt > nextPoll + frequency.interval { nextPoll = polledAt }
            var waitInterval = frequency.interval
            do {
              if frequency != appliedFrequency {
                try sampler.setFrequency(frequency)
                appliedFrequency = frequency
                nextPoll = polledAt
              }
              let reading = try sampler.poll()
              if Task.isCancelled { break }
              if let reading { await self?.accept(reading) } else { await self?.checkStaleness() }
              if reading == nil {
                nextPoll = polledAt
                waitInterval = .milliseconds(50)
              }
            } catch {
              await self?.fail(error.localizedDescription)
            }
            // Keep timer coalescing and view work from accumulating across refreshes.
            nextPoll += waitInterval
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
    if reading.reset { cpuHistory.removeAll(keepingCapacity: true) }
    let previous = cpuHistory.last
    let continuous =
      previous.map { reading.sequence == $0.id + 1 } == true
      && !reading.delayed && !isStale
    let segment = (previous?.segment ?? 0) + (continuous ? 0 : 1)
    cpuHistory.append(
      CPUHistoryPoint(
        id: reading.sequence, time: reading.time, percent: reading.snapshot.cpuPercent,
        segment: segment))
    // Keep one predecessor so the chart can clip a line crossing its left edge.
    while cpuHistory.count > 1 && cpuHistory[1].time <= reading.time - 60 {
      cpuHistory.removeFirst()
    }
    self.reading = reading
    lastReceived = .now
    error = nil
    isStale = false
  }

  func fail(_ message: String) {
    error = message
    isStale = true
  }

  private func checkStaleness() {
    if let lastReceived,
      lastReceived.duration(to: .now) > max(.seconds(3), frequency.interval * 3)
    {
      isStale = true
    }
  }
}
