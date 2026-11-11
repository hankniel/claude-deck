import Foundation

/// One `claude-deck launch` process talking to the app (JSON lines over a unix socket).
final class DeckConnection: Hashable {
    let fd: Int32
    private let lock = NSLock()
    private var closed = false

    init(fd: Int32) { self.fd = fd }

    func send(_ obj: [String: Any]) {
        guard var data = try? JSONSerialization.data(withJSONObject: obj) else { return }
        data.append(0x0A)
        lock.lock(); defer { lock.unlock() }
        guard !closed else { return }
        data.withUnsafeBytes { buf in
            var off = 0
            while off < buf.count {
                let n = Darwin.write(fd, buf.baseAddress! + off, buf.count - off)
                if n <= 0 { break }
                off += n
            }
        }
    }

    func close() {
        lock.lock(); defer { lock.unlock() }
        if !closed { closed = true; Darwin.close(fd) }
    }

    static func == (a: DeckConnection, b: DeckConnection) -> Bool { a === b }
    func hash(into h: inout Hasher) { h.combine(ObjectIdentifier(self)) }
}

final class ControlServer {
    let path: String
    var onMessage: ((DeckConnection, [String: Any]) -> Void)?
    var onClose: ((DeckConnection) -> Void)?
    private var listenFD: Int32 = -1

    init() {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("CCDeck")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        path = dir.appendingPathComponent("ctl-\(getpid()).sock").path
    }

    func start() throws {
        unlink(path)
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw DeckError(message: "socket() failed") }
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let capacity = MemoryLayout.size(ofValue: addr.sun_path)
        guard path.utf8.count < capacity else { throw DeckError(message: "socket path too long") }
        withUnsafeMutablePointer(to: &addr.sun_path) { p in
            p.withMemoryRebound(to: CChar.self, capacity: capacity) { _ = strncpy($0, path, capacity - 1) }
        }
        let ok = withUnsafePointer(to: &addr) { p in
            p.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        guard ok == 0, listen(fd, 16) == 0 else { Darwin.close(fd); throw DeckError(message: "bind/listen failed on \(path)") }
        chmod(path, 0o600)
        listenFD = fd
        Thread.detachNewThread { [weak self] in self?.acceptLoop() }
    }

    func stop() {
        if listenFD >= 0 { Darwin.close(listenFD) }
        unlink(path)
    }

    private func acceptLoop() {
        while true {
            let c = accept(listenFD, nil, nil)
            if c < 0 { if errno == EINTR { continue }; return }
            let conn = DeckConnection(fd: c)
            Thread.detachNewThread { [weak self] in self?.readLoop(conn) }
        }
    }

    private func readLoop(_ conn: DeckConnection) {
        var buffer = Data()
        var chunk = [UInt8](repeating: 0, count: 4096)
        while true {
            let n = read(conn.fd, &chunk, chunk.count)
            if n <= 0 { break }
            buffer.append(contentsOf: chunk[0..<n])
            while let nl = buffer.firstIndex(of: 0x0A) {
                let line = buffer[buffer.startIndex..<nl]
                buffer.removeSubrange(buffer.startIndex...nl)
                if let obj = try? JSONSerialization.jsonObject(with: line) as? [String: Any] {
                    DispatchQueue.main.async { self.onMessage?(conn, obj) }
                }
            }
        }
        DispatchQueue.main.async { self.onClose?(conn) }
        conn.close()
    }
}
