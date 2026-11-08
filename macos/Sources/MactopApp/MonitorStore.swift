import Combine
import Foundation
import MetricsCore

@MainActor
final class MonitorStore: ObservableObject {
  @Published private(set) var reading: MetricsUpdate?
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
          while !Task.isCancelled {
            guard let frequency = await self?.frequency else { return }
            var waitInterval = frequency.interval
            do {
              if frequency != appliedFrequency {
                try sampler.setFrequency(frequency)
                appliedFrequency = frequency
              }
              let reading = try sampler.poll()
              if Task.isCancelled { break }
              if let reading { await self?.accept(reading) } else { await self?.checkStaleness() }
              if reading == nil { waitInterval = .milliseconds(50) }
            } catch {
              await self?.fail(error.localizedDescription)
            }
            await self?.wait(at: frequency, for: waitInterval)
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

  private func wait(at frequency: UpdateFrequency, for interval: Duration) async {
    guard self.frequency == frequency, !Task.isCancelled else { return }
    let current = Task<Void, Never> { try? await Task.sleep(for: interval) }
    delay = current
    await withTaskCancellationHandler {
      await current.value
    } onCancel: {
      current.cancel()
    }
    if delay == current { delay = nil }
  }

  func accept(_ reading: MetricsUpdate) {
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
