import AudioToolbox
import Foundation

/// Decode the core's chunked MP3 response as it arrives, without a temporary file
/// or a web view. All parser/queue ownership lives on the main thread; the audio
/// queue's completion callback returns there before touching that ownership.
@MainActor
final class CarPlayAudioPlayer {
    var onFinished: (() -> Void)?
    private var parser: AudioFileStreamID?
    private var queue: AudioQueueRef?
    private var buffers = 0
    private var queuedBytes = 0
    private var ended = false
    private var failure: OSStatus = noErr

    init() throws {
        let context = Unmanaged.passUnretained(self).toOpaque()
        let status = AudioFileStreamOpen(context, { context, stream, property, _ in
            MainActor.assumeIsolated {
                Unmanaged<CarPlayAudioPlayer>.fromOpaque(context).takeUnretainedValue()
                    .property(stream, property)
            }
        }, { context, byteCount, packetCount, bytes, descriptions in
            MainActor.assumeIsolated {
                Unmanaged<CarPlayAudioPlayer>.fromOpaque(context).takeUnretainedValue()
                    .packets(byteCount, packetCount, bytes, descriptions)
            }
        }, kAudioFileMP3Type, &parser)
        guard status == noErr else { throw audioError(status) }
    }

    func append(_ data: Data) throws {
        guard let parser else { throw audioError(kAudioFileUnspecifiedError) }
        let status = data.withUnsafeBytes { bytes in
            AudioFileStreamParseBytes(parser, UInt32(bytes.count), bytes.baseAddress!, [])
        }
        guard status == noErr, failure == noErr else {
            throw audioError(status != noErr ? status : failure)
        }
    }

    func finish() {
        ended = true
        if let queue { AudioQueueFlush(queue) }
        if buffers == 0 { onFinished?() }
    }

    func stop() {
        if let parser { AudioFileStreamClose(parser) }
        parser = nil
        if let queue { AudioQueueDispose(queue, true) }
        queue = nil
        buffers = 0
        queuedBytes = 0
        onFinished = nil
    }

    private func property(_ stream: AudioFileStreamID, _ property: AudioFileStreamPropertyID) {
        guard property == kAudioFileStreamProperty_ReadyToProducePackets else { return }
        var format = AudioStreamBasicDescription()
        var size = UInt32(MemoryLayout.size(ofValue: format))
        failure = AudioFileStreamGetProperty(stream, kAudioFileStreamProperty_DataFormat, &size, &format)
        guard failure == noErr else { return }
        let context = Unmanaged.passUnretained(self).toOpaque()
        failure = AudioQueueNewOutput(&format, { context, queue, buffer in
            guard let context else { return }
            let size = Int(buffer.pointee.mAudioDataByteSize)
            // Do not retain/dereference the owner off the audio thread. Disposal
            // synchronously ends callbacks; the weak capture guards queued work.
            let owner = Unmanaged<CarPlayAudioPlayer>.fromOpaque(context).takeUnretainedValue()
            Task { @MainActor [weak owner] in
                guard let owner, owner.queue == queue else { return }
                AudioQueueFreeBuffer(queue, buffer)
                owner.buffers -= 1
                owner.queuedBytes -= size
                if owner.ended && owner.buffers == 0 { owner.onFinished?() }
            }
        }, context, nil, nil, 0, &queue)
    }

    private func packets(
        _ byteCount: UInt32, _ packetCount: UInt32, _ bytes: UnsafeRawPointer,
        _ descriptions: UnsafeMutablePointer<AudioStreamPacketDescription>?
    ) {
        guard failure == noErr, let queue else { return }
        // Bound memory when synthesis outruns playback. Fail visibly rather than
        // dropping encoded packets or buffering unbounded audio.
        guard queuedBytes + Int(byteCount) <= 2 * 1024 * 1024 else {
            failure = kAudioQueueErr_EnqueueDuringReset
            return
        }
        var buffer: AudioQueueBufferRef?
        failure = AudioQueueAllocateBuffer(queue, byteCount, &buffer)
        guard failure == noErr, let buffer else { return }
        memcpy(buffer.pointee.mAudioData, bytes, Int(byteCount))
        buffer.pointee.mAudioDataByteSize = byteCount
        failure = AudioQueueEnqueueBuffer(queue, buffer, descriptions == nil ? 0 : packetCount, descriptions)
        guard failure == noErr else {
            AudioQueueFreeBuffer(queue, buffer)
            return
        }
        buffers += 1
        queuedBytes += Int(byteCount)
        failure = AudioQueueStart(queue, nil)
    }

    private func audioError(_ status: OSStatus) -> Error {
        NSError(domain: "HiAgent.CarPlayAudio", code: Int(status), userInfo: [
            NSLocalizedDescriptionKey: "Could not play the agent's audio. Try again."
        ])
    }
}
