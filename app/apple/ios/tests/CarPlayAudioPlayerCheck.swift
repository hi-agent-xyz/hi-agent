import AVFoundation
import Foundation

/// Run on macOS against a short MP3 fixture. This exercises real AudioFileStream
/// decoding, incremental boundaries, draining, and cancellation, not a mock of
/// Apple's audio APIs. See docs/platforms/carplay.md for the command.
@main
struct CarPlayAudioPlayerCheck {
    @MainActor
    static func main() async throws {
        for rate in [16_000.0, 44_100.0, 48_000.0] {
            for channels: AVAudioChannelCount in [1, 2] {
                let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate, channels: channels, interleaved: false)!
                let encoder = try CarPlayPCMEncoder(source: format)
                let count = AVAudioFrameCount(rate / 10)
                let input = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: count)!
                input.frameLength = count
                for channel in 0..<Int(channels) {
                    for sample in 0..<Int(count) { input.floatChannelData![channel][sample] = 0.25 }
                }
                var bytes = 0
                var last = Data()
                for _ in 0..<20 {
                    last = try encoder.encode(input)
                    precondition(last.count % 2 == 0, "PCM sample split")
                    bytes += last.count
                }
                precondition(abs(bytes - 64_000) < 400, "Wrong 16 kHz mono sample count: \(bytes)")
                let sample = last.withUnsafeBytes { $0.loadUnaligned(as: Int16.self) }.littleEndian
                precondition(abs(Int(sample) - 8192) < 200, "Wrong signed little-endian PCM amplitude")
            }
        }
        let data = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1]))
        let player = try CarPlayAudioPlayer()
        var finished = false
        player.onFinished = { finished = true }
        var offset = 0
        let sizes = [1, 13, 509, 4096, 71]
        var step = 0
        while offset < data.count {
            let end = min(data.count, offset + sizes[step % sizes.count])
            try player.append(data.subdata(in: offset..<end))
            offset = end
            step += 1
        }
        player.finish()
        for _ in 0..<100 {
            if finished { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        precondition(finished, "Playback did not drain")
        player.stop()

        let cancelled = try CarPlayAudioPlayer()
        var staleCompletion = false
        cancelled.onFinished = { staleCompletion = true }
        try cancelled.append(data)
        cancelled.stop()
        do {
            try cancelled.append(data)
            preconditionFailure("Stopped player accepted another chunk")
        } catch { }
        try await Task.sleep(for: .milliseconds(300))
        precondition(!staleCompletion, "Cancellation delivered a stale completion")
        print("CarPlay audio checks passed: PCM resampling/mono/Int16, incremental MP3, drain, cancellation")
    }
}
