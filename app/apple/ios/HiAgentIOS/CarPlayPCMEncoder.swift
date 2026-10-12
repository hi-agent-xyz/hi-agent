import AVFoundation
import Foundation

/// One converter per capture tap. Called serially by AVAudioEngine's audio
/// thread; no PCM buffer crosses threads, only copied wire-format bytes do.
final class CarPlayPCMEncoder {
    private let format: AVAudioFormat
    private let converter: AVAudioConverter
    private let inputRate: Double

    init(source: AVAudioFormat) throws {
        guard source.sampleRate > 0, source.channelCount > 0,
              let format = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: 16_000, channels: 1, interleaved: true),
              let converter = AVAudioConverter(from: source, to: format) else {
            throw NSError(domain: "HiAgent.CarPlayPCM", code: 1)
        }
        self.format = format
        self.converter = converter
        inputRate = source.sampleRate
    }

    func encode(_ input: AVAudioPCMBuffer) throws -> Data {
        let capacity = AVAudioFrameCount(ceil(Double(input.frameLength) * 16_000 / inputRate) + 32)
        guard let output = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else {
            throw NSError(domain: "HiAgent.CarPlayPCM", code: 2)
        }
        var supplied = false
        var error: NSError?
        let result = converter.convert(to: output, error: &error) { _, status in
            if supplied { status.pointee = .noDataNow; return nil }
            supplied = true
            status.pointee = .haveData
            return input
        }
        if let error { throw error }
        guard result != .error else { throw NSError(domain: "HiAgent.CarPlayPCM", code: 3) }
        guard output.frameLength > 0, let samples = output.int16ChannelData?[0] else { return Data() }
        return Data(bytes: samples, count: Int(output.frameLength) * 2)
    }
}
