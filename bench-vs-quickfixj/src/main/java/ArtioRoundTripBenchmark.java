import io.aeron.Aeron;
import io.aeron.Publication;
import io.aeron.Subscription;
import io.aeron.archive.Archive;
import io.aeron.archive.ArchiveThreadingMode;
import io.aeron.archive.ArchivingMediaDriver;
import io.aeron.driver.MediaDriver;
import io.aeron.driver.ThreadingMode;
import io.aeron.logbuffer.ControlledFragmentHandler.Action;
import io.aeron.logbuffer.FragmentHandler;
import org.agrona.BufferUtil;
import org.agrona.DirectBuffer;
import org.agrona.IoUtil;
import org.agrona.concurrent.BackoffIdleStrategy;
import org.agrona.concurrent.IdleStrategy;
import org.agrona.concurrent.UnsafeBuffer;

import uk.co.real_logic.artio.ExecType;
import uk.co.real_logic.artio.OrdStatus;
import uk.co.real_logic.artio.Pressure;
import uk.co.real_logic.artio.Side;
import uk.co.real_logic.artio.builder.ExecutionReportEncoder;
import uk.co.real_logic.artio.builder.NewOrderSingleEncoder;
import uk.co.real_logic.artio.decoder.ExecutionReportDecoder;
import uk.co.real_logic.artio.decoder.NewOrderSingleDecoder;
import uk.co.real_logic.artio.engine.EngineConfiguration;
import uk.co.real_logic.artio.engine.FixEngine;
import uk.co.real_logic.artio.fields.DecimalFloat;
import uk.co.real_logic.artio.library.AcquiringSessionExistsHandler;
import uk.co.real_logic.artio.library.FixLibrary;
import uk.co.real_logic.artio.library.LibraryConfiguration;
import uk.co.real_logic.artio.library.LibraryUtil;
import uk.co.real_logic.artio.library.OnMessageInfo;
import uk.co.real_logic.artio.library.SessionConfiguration;
import uk.co.real_logic.artio.library.SessionHandler;
import uk.co.real_logic.artio.messages.DisconnectReason;
import uk.co.real_logic.artio.session.Session;
import uk.co.real_logic.artio.util.MutableAsciiBuffer;

import java.io.File;
import java.nio.ByteBuffer;
import java.util.Arrays;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

import static io.aeron.logbuffer.ControlledFragmentHandler.Action.ABORT;
import static io.aeron.logbuffer.ControlledFragmentHandler.Action.CONTINUE;
import static java.util.Collections.singletonList;

/**
 * End-to-end TCP venue + Aeron IPC round-trip benchmark using Artio
 * (Real Logic's Aeron-native Java FIX engine).
 *
 * Topology (same as the QuickFIX/J version):
 *   Venue (Artio initiator)
 *     ⇄ TCP (127.0.0.1)
 *   Gateway (Artio acceptor)
 *     ⇄ Aeron IPC (separate streams from Artio's own Aeron usage)
 *   Executor
 *
 * Both venue and gateway sessions live on a single FixLibrary polled from one
 * thread — Artio and Session objects are not thread-safe. The Gateway handler's
 * onMessage() synchronously publishes to the executor over Aeron, busy-spins
 * on the reply, then trySends the ExecutionReport back to the venue.
 *
 * Run with: gradle runArtio
 */
public class ArtioRoundTripBenchmark {

    static final int WARMUP     = 10_000;
    static final int ITERATIONS = 100_000;
    static final int TOTAL      = WARMUP + ITERATIONS;

    // Executor IPC streams (distinct from anything Artio uses internally).
    static final int STREAM_GW_TO_EXEC = 2001;
    static final int STREAM_EXEC_TO_GW = 2002;
    static final String EXEC_CHANNEL   = "aeron:ipc";
    static final int FRAME_CAPACITY    = 4096;

    static final int FIX_PORT = findFreePort();

    public static void main(String[] args) throws Exception {
        System.out.println("╔══════════════════════════════════════════════════════════════════╗");
        System.out.println("║     Artio — TCP Venue + Aeron IPC End-to-End Benchmark (Java)   ║");
        System.out.println("╚══════════════════════════════════════════════════════════════════╝");
        System.out.println();
        System.out.println("  Topology:");
        System.out.println("    Venue ⇄ TCP ⇄ Gateway ⇄ Aeron IPC ⇄ Executor");
        System.out.println();
        System.out.println("  Pipeline (measured RTT):");
        System.out.println("    [1] Venue sends FIX NOS over TCP");
        System.out.println("    [2] Gateway parses FIX NOS (Artio decoder)");
        System.out.println("    [3] Gateway encodes order frame");
        System.out.println("    [4] Aeron IPC publish (gateway → executor)");
        System.out.println("    [5] Executor decodes order frame");
        System.out.println("    [6] Executor encodes ExecRpt frame");
        System.out.println("    [7] Aeron IPC publish (executor → gateway)");
        System.out.println("    [8] Gateway decodes ExecRpt frame");
        System.out.println("    [9] Gateway serializes FIX ExecRpt (Artio encoder) → TCP → venue");
        System.out.println();
        System.out.printf("  Warmup:    %,d round-trips (not measured)%n", WARMUP);
        System.out.printf("  Measured:  %,d round-trips%n", ITERATIONS);
        System.out.println();

        final String aeronDir      = new File(System.getProperty("java.io.tmpdir"),
            "artio-bench-aeron-" + ProcessHandle.current().pid()).getAbsolutePath();
        final String archiveDir    = new File(System.getProperty("java.io.tmpdir"),
            "artio-bench-archive-" + ProcessHandle.current().pid()).getAbsolutePath();
        final String logFileDir    = new File(System.getProperty("java.io.tmpdir"),
            "artio-bench-logs-" + ProcessHandle.current().pid()).getAbsolutePath();

        IoUtil.delete(new File(logFileDir), true);

        final String libChannel    = "aeron:ipc";
        final int archiveCtrlPort  = findFreePort();
        final int archiveRespPort  = findFreePort();
        final int archiveReplPort  = findFreePort();
        final String controlReq    = "aeron:udp?endpoint=localhost:" + archiveCtrlPort;
        final String controlResp   = "aeron:udp?endpoint=localhost:" + archiveRespPort;
        final String replication   = "aeron:udp?endpoint=localhost:" + archiveReplPort;

        // ── Media driver + archive ──
        // DEDICATED threading: separate conductor/sender/receiver agents — less context
        // switching between the engine's Aeron hops and our executor Aeron hops.
        final MediaDriver.Context mdCtx = new MediaDriver.Context()
            .aeronDirectoryName(aeronDir)
            .threadingMode(ThreadingMode.DEDICATED)
            .dirDeleteOnStart(true)
            .dirDeleteOnShutdown(true);

        final Archive.Context archiveCtx = new Archive.Context()
            .aeronDirectoryName(aeronDir)
            .archiveDirectoryName(archiveDir)
            .threadingMode(ArchiveThreadingMode.DEDICATED)
            .deleteArchiveOnStart(true)
            .controlChannel(controlReq)
            .replicationChannel(replication);

        try (ArchivingMediaDriver driver = ArchivingMediaDriver.launch(mdCtx, archiveCtx)) {

            // ── Our own Aeron client for the Executor streams ──
            Aeron.Context ourAeronCtx = new Aeron.Context().aeronDirectoryName(aeronDir);
            try (Aeron aeron = Aeron.connect(ourAeronCtx)) {

                Publication  gwPub   = aeron.addPublication(EXEC_CHANNEL, STREAM_GW_TO_EXEC);
                Subscription gwSub   = aeron.addSubscription(EXEC_CHANNEL, STREAM_EXEC_TO_GW);
                Publication  execPub = aeron.addPublication(EXEC_CHANNEL, STREAM_EXEC_TO_GW);
                Subscription execSub = aeron.addSubscription(EXEC_CHANNEL, STREAM_GW_TO_EXEC);

                awaitConnected(gwPub);
                awaitConnected(execPub);

                // ── Executor thread (Aeron-only echo) ──
                CountDownLatch execDone = new CountDownLatch(1);
                Thread execThread = Thread.ofPlatform().start(() -> {
                    try {
                        runExecutor(execPub, execSub);
                    } finally {
                        execDone.countDown();
                    }
                });

                // ── Artio FixEngine (acceptor) ──
                final EngineConfiguration engineCfg = new EngineConfiguration()
                    .bindTo("localhost", FIX_PORT)
                    .libraryAeronChannel(libChannel)
                    .logFileDir(logFileDir)
                    .deleteLogFileDirOnStart(true)
                    .noLogonDisconnectTimeoutInMs(5_000);
                engineCfg.aeronArchiveContext()
                    .controlRequestChannel(controlReq)
                    .controlResponseChannel(controlResp)
                    .aeronDirectoryName(aeronDir);
                engineCfg.aeronContext().aeronDirectoryName(aeronDir);

                try (FixEngine engine = FixEngine.launch(engineCfg)) {
                    runBenchmark(aeronDir, libChannel, gwPub, gwSub);
                }

                // ── Shutdown executor: publish sentinel? Just wait. ──
                execDone.await(30, TimeUnit.SECONDS);
                execThread.join();
            }
        }
    }

    // ── Benchmark core ───────────────────────────────────────────────────────

    static void runBenchmark(String aeronDir, String libChannel, Publication gwPub, Subscription gwSub)
            throws Exception {
        final GatewayHandler gwHandler = new GatewayHandler(gwPub, gwSub);
        final VenueHandler venueHandler = new VenueHandler();

        final LibraryConfiguration libCfg = new LibraryConfiguration()
            .libraryAeronChannels(singletonList(libChannel))
            .sessionAcquireHandler((session, acquiredInfo) -> {
                if (session.isAcceptor()) {
                    gwHandler.attach(session);
                    return gwHandler;
                } else {
                    venueHandler.attach(session);
                    return venueHandler;
                }
            })
            .sessionExistsHandler(new AcquiringSessionExistsHandler(true));
        libCfg.aeronContext().aeronDirectoryName(aeronDir);

        final IdleStrategy idle = new BackoffIdleStrategy(1, 1, 1_000, 1_000_000);

        try (FixLibrary library = FixLibrary.connect(libCfg)) {
            while (!library.isConnected()) {
                library.poll(10);
                idle.idle(0);
            }

            // ── Initiate the Venue outbound session ──
            final SessionConfiguration venueCfg = SessionConfiguration.builder()
                .address("localhost", FIX_PORT)
                .targetCompId("GATEWAY")
                .senderCompId("VENUE")
                .build();

            final Session venueSession = LibraryUtil.initiate(library, venueCfg, 10_000, idle);
            venueHandler.attach(venueSession);

            // ── Drive both sessions until active ──
            long deadline = System.currentTimeMillis() + 10_000;
            while (!venueSession.isActive() || gwHandler.session == null || !gwHandler.session.isActive()) {
                library.poll(10);
                if (System.currentTimeMillis() > deadline) {
                    throw new RuntimeException("session not active in time");
                }
                idle.idle(0);
            }

            // ── Benchmark loop (single thread: library poller + venue sender) ──
            final long[] latencies = new long[ITERATIONS];
            int sent = 0;
            long measuredStart = 0;

            while (sent < TOTAL) {
                // Send NOS (send time captured inside send).
                venueHandler.sendNos(sent);
                sent++;

                // Poll library until venue handler signals reply received.
                while (!venueHandler.replyReceived) {
                    library.poll(10);
                }
                venueHandler.replyReceived = false;

                if (sent == WARMUP) {
                    measuredStart = System.nanoTime();
                } else if (sent > WARMUP) {
                    latencies[sent - WARMUP - 1] = venueHandler.lastRttNs;
                }
            }

            long measuredElapsed = System.nanoTime() - measuredStart;
            printResults(latencies, measuredElapsed);
        }
    }

    // ── Venue handler (initiator side) ───────────────────────────────────────

    static final byte[] FIX_TIMESTAMP = "20260321-10:00:00.123".getBytes(java.nio.charset.StandardCharsets.US_ASCII);
    static final byte[] SYMBOL_AAPL   = "AAPL".getBytes(java.nio.charset.StandardCharsets.US_ASCII);

    static class VenueHandler implements SessionHandler {
        Session session;
        final NewOrderSingleEncoder nos = new NewOrderSingleEncoder();
        // Pre-allocated ClOrdID buffer: "ORD-00000000" — last 8 bytes are the sequence.
        final byte[] clOrdIdBuf = new byte[] {'O','R','D','-','0','0','0','0','0','0','0','0'};
        final UnsafeBuffer clOrdIdEnc = new UnsafeBuffer(clOrdIdBuf);
        long sendTimeNs;
        long lastRttNs;
        boolean replyReceived;

        void attach(Session s) { this.session = s; }

        void sendNos(int seq) {
            // Zero-alloc: write 8 ASCII digits of seq into positions 4..11.
            int n = seq;
            for (int i = 11; i >= 4; i--) {
                clOrdIdBuf[i] = (byte) ('0' + (n % 10));
                n /= 10;
            }

            nos.resetMessage();
            nos.clOrdID(clOrdIdBuf, 0, 12);
            nos.side(Side.BUY);
            nos.transactTime(FIX_TIMESTAMP);
            nos.ordType(uk.co.real_logic.artio.OrdType.LIMIT);
            nos.instrument().symbol(SYMBOL_AAPL, 0, SYMBOL_AAPL.length);
            nos.orderQtyData().orderQty(10_000L, 0);
            nos.price(17855L, 2);

            sendTimeNs = System.nanoTime();
            long pos;
            do {
                pos = session.trySend(nos);
            } while (Pressure.isBackPressured(pos));
        }

        @Override
        public Action onMessage(DirectBuffer buffer, int offset, int length, int libraryId,
                                Session session, int sequenceIndex, long messageType,
                                long timestampInNs, long position, OnMessageInfo messageInfo) {
            if (messageType == ExecutionReportDecoder.MESSAGE_TYPE) {
                lastRttNs = System.nanoTime() - sendTimeNs;
                replyReceived = true;
            }
            return CONTINUE;
        }

        @Override public void onTimeout(int libraryId, Session session) {}
        @Override public void onSlowStatus(int libraryId, Session session, boolean hasBecomeSlow) {}
        @Override public Action onDisconnect(int libraryId, Session session, DisconnectReason reason) { return CONTINUE; }
        @Override public void onSessionStart(Session session) {}
    }

    // ── Gateway handler (acceptor side) ──────────────────────────────────────

    static class GatewayHandler implements SessionHandler {
        Session session;
        final Publication gwPub;
        final Subscription gwSub;

        final MutableAsciiBuffer asciiBuffer = new MutableAsciiBuffer();
        final NewOrderSingleDecoder nosDecoder = new NewOrderSingleDecoder();
        final ExecutionReportEncoder erEncoder = new ExecutionReportEncoder();

        // Pre-allocated executor frame buffers.
        final UnsafeBuffer gwSendBuf = new UnsafeBuffer(
            BufferUtil.allocateDirectAligned(FRAME_CAPACITY, 64));
        final ByteBuffer orderFrame = ByteBuffer.allocateDirect(FRAME_CAPACITY);
        final byte[] gwRecvScratch = new byte[FRAME_CAPACITY];
        final int[] gwRecvLen = {0};
        final FragmentHandler gwFragmentHandler;

        // Reusable encode scratch for exec id / order id.
        final byte[] execIdBuf = new byte[32];
        final UnsafeBuffer execIdEnc = new UnsafeBuffer(execIdBuf);
        // Pre-filled orderId — constant across iterations.
        final byte[] orderIdBuf = "NYSE-ORD-001".getBytes(java.nio.charset.StandardCharsets.US_ASCII);
        // Scratch for symbol read out of the executor frame.
        final byte[] symbolScratch = new byte[64];
        int symbolScratchLen = 0;
        long execIdSeq = 0;

        GatewayHandler(Publication gwPub, Subscription gwSub) {
            this.gwPub = gwPub;
            this.gwSub = gwSub;
            this.gwFragmentHandler = (buffer, offset, length, header) -> {
                buffer.getBytes(offset, gwRecvScratch, 0, length);
                gwRecvLen[0] = length;
            };
        }

        void attach(Session s) { this.session = s; }

        @Override
        public Action onMessage(DirectBuffer buffer, int offset, int length, int libraryId,
                                Session session, int sequenceIndex, long messageType,
                                long timestampInNs, long position, OnMessageInfo messageInfo) {
            if (messageType != NewOrderSingleDecoder.MESSAGE_TYPE) return CONTINUE;

            // [2] Parse inbound NOS.
            asciiBuffer.wrap(buffer);
            nosDecoder.decode(asciiBuffer, offset, length);

            // [3] Pack order frame for executor.
            orderFrame.clear();
            packOrderFrame(nosDecoder, orderFrame);
            orderFrame.flip();
            int frameLen = orderFrame.remaining();
            gwSendBuf.wrap(orderFrame, 0, frameLen);

            // [4] Publish to executor.
            publishWithRetry(gwPub, gwSendBuf, frameLen);

            // [7] Busy-spin on executor reply.
            gwRecvLen[0] = 0;
            while (gwRecvLen[0] == 0) {
                gwSub.poll(gwFragmentHandler, 1);
            }

            // [8] Decode executor reply — zero-alloc walk through gwRecvScratch.
            // Frame layout (from runExecutor): execId | orderId | clOrdId | symbol | side | qty | price.
            int p = 0;
            p = skipField(gwRecvScratch, p);          // execId (unused here)
            p = skipField(gwRecvScratch, p);          // orderId (we use constant orderIdBuf)
            p = skipField(gwRecvScratch, p);          // clOrdId (unused here)
            // symbol — read into scratch
            int symLen = gwRecvScratch[p++] & 0xFF;
            System.arraycopy(gwRecvScratch, p, symbolScratch, 0, symLen);
            symbolScratchLen = symLen;
            p += symLen;
            // side/qty/price are present but we don't need them for the minimal encoder.

            // [9] Build and send FIX ExecRpt via Artio encoder (zero-alloc).
            int execIdLen = execIdEnc.putLongAscii(0, ++execIdSeq);

            erEncoder.resetMessage();
            erEncoder.orderID(orderIdBuf, 0, orderIdBuf.length);
            erEncoder.execID(execIdBuf, 0, execIdLen);
            erEncoder.execType(ExecType.FILL);
            erEncoder.ordStatus(OrdStatus.FILLED);
            erEncoder.side(Side.BUY);
            erEncoder.transactTime(FIX_TIMESTAMP);
            erEncoder.instrument().symbol(symbolScratch, 0, symbolScratchLen);

            long pos = session.trySend(erEncoder);
            if (Pressure.isBackPressured(pos)) {
                --execIdSeq;
                return ABORT;
            }
            return CONTINUE;
        }

        static int skipField(byte[] buf, int p) {
            int len = buf[p++] & 0xFF;
            return p + len;
        }

        @Override public void onTimeout(int libraryId, Session session) {}
        @Override public void onSlowStatus(int libraryId, Session session, boolean hasBecomeSlow) {}
        @Override public Action onDisconnect(int libraryId, Session session, DisconnectReason reason) { return CONTINUE; }
        @Override public void onSessionStart(Session session) {}
    }

    // ── Frame helpers (shared with AeronRoundTripBenchmark codec) ────────────

    static void packOrderFrame(NewOrderSingleDecoder nos, ByteBuffer out) {
        char[] clOrdId = nos.clOrdID();
        int clOrdIdLen = nos.clOrdIDLength();
        char[] symbol  = nos.symbol();
        int symbolLen  = nos.symbolLength();

        writeFieldChars(out, clOrdId, clOrdIdLen);
        writeFieldChars(out, symbol, symbolLen);
        out.put((byte) nos.sideAsEnum().representation());
        DecimalFloat qty = nos.orderQty();
        out.putDouble(decimalToDouble(qty));
        DecimalFloat px = nos.price();
        out.putDouble(decimalToDouble(px));
    }

    static double decimalToDouble(DecimalFloat d) {
        double v = d.value();
        int s = d.scale();
        for (int i = 0; i < s; i++) v /= 10.0;
        return v;
    }

    static void writeField(ByteBuffer buf, String s) {
        byte[] bytes = s.getBytes(java.nio.charset.StandardCharsets.UTF_8);
        buf.put((byte) bytes.length);
        buf.put(bytes);
    }

    static void writeFieldChars(ByteBuffer buf, char[] chars, int len) {
        buf.put((byte) len);
        for (int i = 0; i < len; i++) {
            buf.put((byte) chars[i]);
        }
    }

    static String readField(ByteBuffer buf) {
        int len = buf.get() & 0xFF;
        byte[] bytes = new byte[len];
        buf.get(bytes);
        return new String(bytes, java.nio.charset.StandardCharsets.UTF_8);
    }

    // ── Executor loop (shared logic with AeronRoundTripBenchmark) ────────────

    static void runExecutor(Publication execPub, Subscription execSub) {
        UnsafeBuffer execSendBuf = new UnsafeBuffer(
            BufferUtil.allocateDirectAligned(FRAME_CAPACITY, 64));
        byte[] scratch = new byte[FRAME_CAPACITY];
        byte[] recvScratch = new byte[FRAME_CAPACITY];
        int[] recvLen = {0};
        FragmentHandler execHandler = (buffer, offset, length, header) -> {
            buffer.getBytes(offset, recvScratch, 0, length);
            recvLen[0] = length;
        };

        final byte[] EXEC_ID_BYTES = "EXEC-001".getBytes(java.nio.charset.StandardCharsets.US_ASCII);
        final byte[] ORDER_ID_BYTES = "NYSE-ORD-001".getBytes(java.nio.charset.StandardCharsets.US_ASCII);

        for (int i = 0; i < TOTAL; i++) {
            recvLen[0] = 0;
            while (recvLen[0] == 0) {
                execSub.poll(execHandler, 1);
            }

            // Zero-alloc walk over inbound frame: clOrdId | symbol | side | qty | price.
            int rp = 0;
            int clOrdIdLen = recvScratch[rp++] & 0xFF;
            int clOrdIdOff = rp;
            rp += clOrdIdLen;
            int symbolLen = recvScratch[rp++] & 0xFF;
            int symbolOff = rp;
            rp += symbolLen;
            byte side = recvScratch[rp++];
            // qty/price: 16 bytes, copied verbatim into the reply below.
            int qtyPriceOff = rp;

            // Zero-alloc write into scratch: execId | orderId | clOrdId | symbol | side | qty | price.
            int wp = 0;
            scratch[wp++] = (byte) EXEC_ID_BYTES.length;
            System.arraycopy(EXEC_ID_BYTES, 0, scratch, wp, EXEC_ID_BYTES.length);
            wp += EXEC_ID_BYTES.length;
            scratch[wp++] = (byte) ORDER_ID_BYTES.length;
            System.arraycopy(ORDER_ID_BYTES, 0, scratch, wp, ORDER_ID_BYTES.length);
            wp += ORDER_ID_BYTES.length;
            scratch[wp++] = (byte) clOrdIdLen;
            System.arraycopy(recvScratch, clOrdIdOff, scratch, wp, clOrdIdLen);
            wp += clOrdIdLen;
            scratch[wp++] = (byte) symbolLen;
            System.arraycopy(recvScratch, symbolOff, scratch, wp, symbolLen);
            wp += symbolLen;
            scratch[wp++] = side;
            System.arraycopy(recvScratch, qtyPriceOff, scratch, wp, 16);
            wp += 16;

            execSendBuf.wrap(scratch, 0, wp);
            publishWithRetry(execPub, execSendBuf, wp);
        }
    }

    // ── Aeron helpers ────────────────────────────────────────────────────────

    static void publishWithRetry(Publication pub, UnsafeBuffer buf, int length) {
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5);
        while (true) {
            long result = pub.offer(buf, 0, length);
            if (result > 0) return;
            if ((result == Publication.BACK_PRESSURED || result == Publication.ADMIN_ACTION)
                    && System.nanoTime() < deadline) {
                Thread.yield();
                continue;
            }
            throw new RuntimeException("Aeron offer failed: " + result);
        }
    }

    static void awaitConnected(Publication pub) throws InterruptedException {
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(10);
        while (!pub.isConnected()) {
            Thread.sleep(1);
            if (System.nanoTime() > deadline)
                throw new RuntimeException("Aeron connection timeout");
        }
    }

    static int findFreePort() {
        try (java.net.ServerSocket s = new java.net.ServerSocket(0)) {
            return s.getLocalPort();
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
    }

    // ── Reporting ────────────────────────────────────────────────────────────

    static void printResults(long[] latencies, long totalNs) {
        long[] sorted = Arrays.copyOf(latencies, latencies.length);
        Arrays.sort(sorted);
        int n = sorted.length;
        long sum = 0;
        for (long v : sorted) sum += v;

        System.out.println();
        System.out.println("  Round-trip latency (venue FIX NOS → venue FIX ExecRpt over TCP):");
        System.out.printf("    min:    %10.1f µs%n", sorted[0]                                  / 1_000.0);
        System.out.printf("    mean:   %10.1f µs%n", (sum / (double) n)                         / 1_000.0);
        System.out.printf("    p50:    %10.1f µs%n", sorted[n * 50  / 100]                      / 1_000.0);
        System.out.printf("    p90:    %10.1f µs%n", sorted[n * 90  / 100]                      / 1_000.0);
        System.out.printf("    p99:    %10.1f µs%n", sorted[n * 99  / 100]                      / 1_000.0);
        System.out.printf("    p99.9:  %10.1f µs%n", sorted[Math.min(n - 1, n * 999 / 1000)]    / 1_000.0);
        System.out.printf("    max:    %10.1f µs%n", sorted[n - 1]                              / 1_000.0);
        System.out.println();
        System.out.printf("  Throughput:                %.0f round-trips/sec%n",
            (double) n / (totalNs / 1_000_000_000.0));
    }
}
