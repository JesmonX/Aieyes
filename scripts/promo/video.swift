// Promo video encoder built on AVFoundation, so no ffmpeg install is needed.
//
//   video encode --output out.mp4 --fps 30 --width 1920 --height 1080 < frames
//       stdin carries frames as a 4-byte big-endian length followed by PNG/JPEG bytes.
//   video mux --video in.mp4 --audio in.m4a --output out.mp4
//       copies the H.264 track and the AAC track into one MP4 without re-encoding.
import AVFoundation
import CoreGraphics
import Foundation
import ImageIO

let arguments = CommandLine.arguments
func option(_ name: String) -> String? {
    guard let index = arguments.firstIndex(of: name), arguments.indices.contains(index + 1) else { return nil }
    return arguments[index + 1]
}
func fail(_ message: String) -> Never { FileHandle.standardError.write((message + "\n").data(using: .utf8)!); exit(1) }
func required(_ name: String) -> String {
    guard let value = option(name) else { fail("Missing " + name) }
    return value
}

func readExactly(_ count: Int) -> Data? {
    var data = Data(count: count), filled = 0
    data.withUnsafeMutableBytes { buffer in
        while filled < count {
            let read = fread(buffer.baseAddress! + filled, 1, count - filled, stdin)
            if read <= 0 { break }
            filled += read
        }
    }
    return filled == count ? data : nil
}

func encode() async {
    let output = URL(fileURLWithPath: required("--output"))
    let fps = Int32(option("--fps") ?? "30")!, width = Int(option("--width") ?? "1920")!, height = Int(option("--height") ?? "1080")!
    let bitrate = Int(option("--bitrate") ?? "20000000")!
    try? FileManager.default.removeItem(at: output)
    let writer: AVAssetWriter
    do { writer = try AVAssetWriter(outputURL: output, fileType: .mp4) } catch { fail(error.localizedDescription) }
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
        AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: width, AVVideoHeightKey: height,
        AVVideoCompressionPropertiesKey: [AVVideoAverageBitRateKey: bitrate, AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
                                          AVVideoMaxKeyFrameIntervalKey: Int(fps) * 2, AVVideoExpectedSourceFrameRateKey: Int(fps)],
        AVVideoColorPropertiesKey: [AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_709_2, AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_709_2,
                                    AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2]])
    input.expectsMediaDataInRealTime = false
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA, kCVPixelBufferWidthKey as String: width, kCVPixelBufferHeightKey as String: height])
    writer.add(input)
    guard writer.startWriting() else { fail(writer.error?.localizedDescription ?? "Cannot start writing") }
    writer.startSession(atSourceTime: .zero)
    let space = CGColorSpace(name: CGColorSpace.sRGB)!
    var index: Int64 = 0
    while let header = readExactly(4) {
        let length = header.reduce(0) { $0 << 8 | Int($1) }
        guard let bytes = readExactly(length), let source = CGImageSourceCreateWithData(bytes as CFData, nil),
              let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { fail("Unreadable frame \(index)") }
        while !input.isReadyForMoreMediaData { try? await Task.sleep(nanoseconds: 2_000_000) }
        guard let pool = adaptor.pixelBufferPool else { fail(writer.error?.localizedDescription ?? "No pixel buffer pool") }
        var buffer: CVPixelBuffer?
        CVPixelBufferPoolCreatePixelBuffer(nil, pool, &buffer)
        guard let buffer else { fail("No pixel buffer") }
        CVPixelBufferLockBaseAddress(buffer, [])
        let context = CGContext(data: CVPixelBufferGetBaseAddress(buffer), width: width, height: height, bitsPerComponent: 8,
                                bytesPerRow: CVPixelBufferGetBytesPerRow(buffer), space: space,
                                bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)!
        context.interpolationQuality = .high
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        CVPixelBufferUnlockBaseAddress(buffer, [])
        CVBufferSetAttachment(buffer, kCVImageBufferColorPrimariesKey, kCVImageBufferColorPrimaries_ITU_R_709_2, .shouldPropagate)
        CVBufferSetAttachment(buffer, kCVImageBufferTransferFunctionKey, kCVImageBufferTransferFunction_ITU_R_709_2, .shouldPropagate)
        CVBufferSetAttachment(buffer, kCVImageBufferYCbCrMatrixKey, kCVImageBufferYCbCrMatrix_ITU_R_709_2, .shouldPropagate)
        guard adaptor.append(buffer, withPresentationTime: CMTime(value: index, timescale: fps)) else { fail(writer.error?.localizedDescription ?? "Append failed") }
        index += 1
    }
    input.markAsFinished()
    await writer.finishWriting()
    guard writer.status == .completed else { fail(writer.error?.localizedDescription ?? "Encoding failed") }
    print("Encoded \(index) frames into \(output.path)")
}

func mux() async {
    let video = AVURLAsset(url: URL(fileURLWithPath: required("--video"))), audio = AVURLAsset(url: URL(fileURLWithPath: required("--audio")))
    let output = URL(fileURLWithPath: required("--output"))
    try? FileManager.default.removeItem(at: output)
    do {
        let composition = AVMutableComposition()
        guard let source = try await video.loadTracks(withMediaType: .video).first, let sound = try await audio.loadTracks(withMediaType: .audio).first else { fail("Missing track") }
        let duration = try await video.load(.duration), soundDuration = try await audio.load(.duration)
        let range = CMTimeRange(start: .zero, duration: duration)
        try composition.addMutableTrack(withMediaType: .video, preferredTrackID: kCMPersistentTrackID_Invalid)!.insertTimeRange(range, of: source, at: .zero)
        try composition.addMutableTrack(withMediaType: .audio, preferredTrackID: kCMPersistentTrackID_Invalid)!
            .insertTimeRange(CMTimeRange(start: .zero, duration: CMTimeMinimum(duration, soundDuration)), of: sound, at: .zero)
        guard let export = AVAssetExportSession(asset: composition, presetName: AVAssetExportPresetPassthrough) else { fail("Cannot export") }
        export.shouldOptimizeForNetworkUse = true
        try await export.export(to: output, as: .mp4)
        print("Muxed \(output.path)")
    } catch { fail(error.localizedDescription) }
}

switch arguments.dropFirst().first {
case "encode": await encode()
case "mux": await mux()
default: fail("Usage: video encode|mux …")
}
