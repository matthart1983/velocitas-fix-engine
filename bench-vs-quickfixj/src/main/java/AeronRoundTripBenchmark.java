import io.aeron.Aeron;
import io.aeron.Publication;
import io.aeron.Subscription;
import io.aeron.driver.MediaDriver;
import io.aeron.logbuffer.FragmentHandler;
import org.agrona.BufferUtil;
import org.agrona.concurrent.UnsafeBuffer;

import quickfix.*;
import quickfix.field.*;
import quickfix.fix44.ExecutionReport;
import quickfix.fix44.NewOrderSingle;

import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

/**
 * End-to-end TCP venue + Aeron IPC round-trip benchmark.
 *
 * Topology:
 *   Venue (QuickFIX/J SocketInitiator)
 *     ⇄ TCP (127.0.0.1)
 *   Gateway (QuickFIX/J SocketAcceptor)
 *     ⇄ Aeron IPC
 *   Executor
 *
 * Pipeline (what the RTT timer on the venue measures):
 *   [1] Venue sends FIX NOS over TCP
 *   [2] Gateway QuickFIX/J parses NOS
 *   [3] Gateway extracts fields → encodes order frame
 *   [4] Aeron IPC publish (gateway → executor)
 *   [5] Executor decodes order frame
 *   [6] Executor encodes ExecRpt frame
 *   [7] Aeron IPC publish (executor → gateway)
 *   [8] Gateway decodes ExecRpt frame
 *   [9] Gateway QuickFIX/J serializes FIX ExecRpt → TCP → venue
 *
 * Run with: gradle runE2E
 */
public class AeronRoundTripBenchmark {

    static final int WARMUP     = 10_000;
    static final int ITERATIONS = 100_000;
    static final int TOTAL      = WARMUP + ITERATIONS;

    static final int STREAM_GW_TO_EXEC = 1001;
    static final int STREAM_EXEC_TO_GW = 1002;
    static final String CHANNEL        = "aeron:ipc";

    static final int FRAME_CAPACITY = 4096;

    // Shared state between initiator and acceptor threads.
    static final CountDownLatch doneLatch = new CountDownLatch(1);
    static final List<Long> latencies = Collections.synchronizedList(new ArrayList<>(ITERATIONS));
    static volatile long sendTimeNs;
    static volatile int sentCount;
    static volatile long measuredStartNs;
    static volatile long sessionElapsedNs;

    public static void main(String[] args) throws Exception {
        System.out.println("╔══════════════════════════════════════════════════════════════════╗");
        System.out.println("║  QuickFIX/J — TCP Venue + Aeron IPC End-to-End Benchmark (Java) ║");
        System.out.println("╚══════════════════════════════════════════════════════════════════╝");
        System.out.println();
        System.out.println("  Topology:");
        System.out.println("    Venue ⇄ TCP ⇄ Gateway ⇄ Aeron IPC ⇄ Executor");
        System.out.println();
        System.out.println("  Pipeline (measured RTT):");
        System.out.println("    [1] Venue sends FIX NOS over TCP");
        System.out.println("    [2] Gateway parses FIX NOS");
        System.out.println("    [3] Gateway encodes order frame");
        System.out.println("    [4] Aeron IPC publish (gateway → executor)");
        System.out.println("    [5] Executor decodes order frame");
        System.out.println("    [6] Executor encodes ExecRpt frame");
        System.out.println("    [7] Aeron IPC publish (executor → gateway)");
        System.out.println("    [8] Gateway decodes ExecRpt frame");
        System.out.println("    [9] Gateway serializes FIX ExecRpt → TCP → venue");
        System.out.println();
        System.out.printf("  Warmup:    %,d round-trips (not measured)%n", WARMUP);
        System.out.printf("  Measured:  %,d round-trips%n", ITERATIONS);
        System.out.println();

        MediaDriver.Context driverCtx = new MediaDriver.Context()
            .dirDeleteOnStart(true)
            .dirDeleteOnShutdown(true);
        try (MediaDriver driver = MediaDriver.launchEmbedded(driverCtx)) {
            Aeron.Context aeronCtx = new Aeron.Context()
                .aeronDirectoryName(driver.aeronDirectoryName());
            try (Aeron aeron = Aeron.connect(aeronCtx)) {
                runBenchmark(aeron);
            }
        }
    }

    static void runBenchmark(Aeron aeron) throws Exception {
        // ── Aeron links ──
        Publication  gwPub   = aeron.addPublication(CHANNEL, STREAM_GW_TO_EXEC);
        Subscription gwSub   = aeron.addSubscription(CHANNEL, STREAM_EXEC_TO_GW);
        Publication  execPub = aeron.addPublication(CHANNEL, STREAM_EXEC_TO_GW);
        Subscription execSub = aeron.addSubscription(CHANNEL, STREAM_GW_TO_EXEC);

        awaitConnected(gwPub);
        awaitConnected(execPub);

        // ── Executor thread ──
        CountDownLatch execDone = new CountDownLatch(1);
        Thread execThread = Thread.ofPlatform().start(() -> {
            runExecutor(execPub, execSub);
            execDone.countDown();
        });

        // ── QuickFIX/J acceptor (gateway) ──
        int port = findFreePort();

        SessionSettings acceptorSettings = new SessionSettings();
        Dictionary acceptorDefaults = new Dictionary();
        acceptorDefaults.setString("ConnectionType", "acceptor");
        acceptorDefaults.setString("SocketAcceptPort", String.valueOf(port));
        acceptorDefaults.setString("StartTime", "00:00:00");
        acceptorDefaults.setString("EndTime", "23:59:59");
        acceptorDefaults.setString("HeartBtInt", "30");
        acceptorDefaults.setString("ResetOnLogon", "Y");
        acceptorDefaults.setString("UseDataDictionary", "N");
        SessionID acceptorId = new SessionID("FIX.4.4", "NYSE", "BANK_OMS");
        acceptorSettings.set(acceptorDefaults);
        acceptorSettings.set(acceptorId, new Dictionary());

        MessageStoreFactory storeFactory = new MemoryStoreFactory();
        LogFactory logFactory = new ScreenLogFactory(false, false, false);

        GatewayApp gatewayApp = new GatewayApp(gwPub, gwSub);
        SocketAcceptor acceptor = new SocketAcceptor(
            gatewayApp, storeFactory, acceptorSettings, logFactory, new DefaultMessageFactory()
        );
        acceptor.start();

        // ── QuickFIX/J initiator (venue) ──
        SessionSettings initiatorSettings = new SessionSettings();
        Dictionary initiatorDefaults = new Dictionary();
        initiatorDefaults.setString("ConnectionType", "initiator");
        initiatorDefaults.setString("SocketConnectHost", "127.0.0.1");
        initiatorDefaults.setString("SocketConnectPort", String.valueOf(port));
        initiatorDefaults.setString("StartTime", "00:00:00");
        initiatorDefaults.setString("EndTime", "23:59:59");
        initiatorDefaults.setString("HeartBtInt", "30");
        initiatorDefaults.setString("ResetOnLogon", "Y");
        initiatorDefaults.setString("UseDataDictionary", "N");
        SessionID initiatorId = new SessionID("FIX.4.4", "BANK_OMS", "NYSE");
        initiatorSettings.set(initiatorDefaults);
        initiatorSettings.set(initiatorId, new Dictionary());

        VenueApp venueApp = new VenueApp(initiatorId);
        SocketInitiator initiator = new SocketInitiator(
            venueApp, storeFactory, initiatorSettings, logFactory, new DefaultMessageFactory()
        );
        initiator.start();

        doneLatch.await(300, TimeUnit.SECONDS);

        initiator.stop();
        acceptor.stop();
        execDone.await(30, TimeUnit.SECONDS);
        execThread.join();

        printResults();
    }

    // ── Executor (Aeron loop) ────────────────────────────────────────────────

    static void runExecutor(Publication execPub, Subscription execSub) {
        UnsafeBuffer execSendBuf = new UnsafeBuffer(
            BufferUtil.allocateDirectAligned(FRAME_CAPACITY, 64));
        ByteBuffer scratch = ByteBuffer.allocateDirect(FRAME_CAPACITY);
        byte[] recvScratch = new byte[FRAME_CAPACITY];
        int[] recvLen = {0};
        FragmentHandler execHandler = (buffer, offset, length, header) -> {
            buffer.getBytes(offset, recvScratch, 0, length);
            recvLen[0] = length;
        };

        for (int i = 0; i < TOTAL; i++) {
            recvLen[0] = 0;
            while (recvLen[0] == 0) {
                execSub.poll(execHandler, 1);
            }
            ByteBuffer orderFrame = ByteBuffer.wrap(recvScratch, 0, recvLen[0]);

            scratch.clear();
            buildExecRptFrame(orderFrame, scratch);
            scratch.flip();

            int replyLen = scratch.remaining();
            execSendBuf.wrap(scratch, 0, replyLen);
            publishWithRetry(execPub, execSendBuf, replyLen);
        }
    }

    // ── Gateway (QuickFIX/J acceptor + Aeron) ────────────────────────────────

    static class GatewayApp implements Application {
        final Publication gwPub;
        final Subscription gwSub;
        final UnsafeBuffer gwSendBuf = new UnsafeBuffer(
            BufferUtil.allocateDirectAligned(FRAME_CAPACITY, 64));
        final ByteBuffer orderFrame = ByteBuffer.allocateDirect(FRAME_CAPACITY);
        final byte[] gwRecvScratch = new byte[FRAME_CAPACITY];
        final int[] gwRecvLen = {0};
        final FragmentHandler gwHandler;

        GatewayApp(Publication gwPub, Subscription gwSub) {
            this.gwPub = gwPub;
            this.gwSub = gwSub;
            this.gwHandler = (buffer, offset, length, header) -> {
                buffer.getBytes(offset, gwRecvScratch, 0, length);
                gwRecvLen[0] = length;
            };
        }

        @Override public void onCreate(SessionID sessionId) {}
        @Override public void onLogon(SessionID sessionId) {}
        @Override public void onLogout(SessionID sessionId) {}
        @Override public void toAdmin(Message message, SessionID sessionId) {}
        @Override public void fromAdmin(Message message, SessionID sessionId) {}
        @Override public void toApp(Message message, SessionID sessionId) {}

        @Override
        public void fromApp(Message message, SessionID sessionId)
                throws FieldNotFound, UnsupportedMessageType, IncorrectTagValue {
            String msgType;
            try {
                msgType = message.getHeader().getString(MsgType.FIELD);
            } catch (FieldNotFound e) { return; }
            if (!"D".equals(msgType)) return;

            // [3] Extract fields → encode order frame.
            orderFrame.clear();
            packOrderFrame(message, orderFrame);
            orderFrame.flip();
            int frameLen = orderFrame.remaining();
            gwSendBuf.wrap(orderFrame, 0, frameLen);

            // [4] Publish to executor.
            publishWithRetry(gwPub, gwSendBuf, frameLen);

            // [7] Receive reply from executor — busy-spin.
            gwRecvLen[0] = 0;
            while (gwRecvLen[0] == 0) {
                gwSub.poll(gwHandler, 1);
            }
            ByteBuffer replyFrame = ByteBuffer.wrap(gwRecvScratch, 0, gwRecvLen[0]);

            // [8] Decode ExecRpt frame.
            String execId  = readField(replyFrame);
            String orderId = readField(replyFrame);
            String clOrdId = readField(replyFrame);
            String symbol  = readField(replyFrame);
            char   side    = (char) replyFrame.get();
            double qty     = replyFrame.getDouble();
            double price   = replyFrame.getDouble();

            // [9] Build FIX ExecRpt and send over TCP.
            ExecutionReport rpt = new ExecutionReport(
                new OrderID(orderId),
                new ExecID(execId),
                new ExecType(ExecType.FILL),
                new OrdStatus(OrdStatus.FILLED),
                new Side(side),
                new LeavesQty(0),
                new CumQty(qty),
                new AvgPx(price)
            );
            rpt.set(new ClOrdID(clOrdId));
            rpt.set(new Symbol(symbol));
            rpt.set(new OrderQty(qty));
            rpt.set(new LastPx(price));
            rpt.set(new LastQty(qty));

            try {
                Session.sendToTarget(rpt, sessionId);
            } catch (SessionNotFound e) {
                throw new RuntimeException(e);
            }
        }
    }

    // ── Venue (QuickFIX/J initiator) ─────────────────────────────────────────

    static class VenueApp implements Application {
        final SessionID sessionId;

        VenueApp(SessionID sessionId) {
            this.sessionId = sessionId;
        }

        @Override public void onCreate(SessionID sessionId) {}
        @Override public void onLogon(SessionID sessionId) {
            sendNos();
        }
        @Override public void onLogout(SessionID sessionId) {}
        @Override public void toAdmin(Message message, SessionID sessionId) {}
        @Override public void fromAdmin(Message message, SessionID sessionId) {}
        @Override public void toApp(Message message, SessionID sessionId) {}

        @Override
        public void fromApp(Message message, SessionID sessionId)
                throws FieldNotFound, UnsupportedMessageType, IncorrectTagValue {
            String msgType = message.getHeader().getString(MsgType.FIELD);
            if (!"8".equals(msgType)) return;

            long rtt = System.nanoTime() - sendTimeNs;
            if (sentCount > WARMUP) {
                latencies.add(rtt);
            } else if (sentCount == WARMUP) {
                measuredStartNs = System.nanoTime();
            }

            if (sentCount < TOTAL) {
                sendNos();
            } else {
                sessionElapsedNs = System.nanoTime() - measuredStartNs;
                doneLatch.countDown();
            }
        }

        private void sendNos() {
            try {
                NewOrderSingle nos = new NewOrderSingle(
                    new ClOrdID("ORD-" + String.format("%08d", sentCount)),
                    new Side(Side.BUY),
                    new TransactTime(),
                    new OrdType(OrdType.LIMIT)
                );
                nos.set(new Symbol("AAPL"));
                nos.set(new OrderQty(10000));
                nos.set(new Price(178.55));

                sentCount++;
                sendTimeNs = System.nanoTime();
                Session.sendToTarget(nos, sessionId);
            } catch (SessionNotFound e) {
                throw new RuntimeException(e);
            }
        }
    }

    // ── Frame helpers ─────────────────────────────────────────────────────────

    static void packOrderFrame(Message nos, ByteBuffer out) {
        try {
            writeField(out, nos.getString(ClOrdID.FIELD));
            writeField(out, nos.getString(Symbol.FIELD));
            out.put((byte) nos.getChar(Side.FIELD));
            out.putDouble(nos.getDouble(OrderQty.FIELD));
            out.putDouble(nos.getDouble(Price.FIELD));
            writeField(out, nos.getHeader().getString(SenderCompID.FIELD));
            writeField(out, nos.getHeader().getString(TargetCompID.FIELD));
        } catch (FieldNotFound e) {
            throw new RuntimeException(e);
        }
    }

    static void buildExecRptFrame(ByteBuffer orderFrame, ByteBuffer out) {
        String clOrdId  = readField(orderFrame);
        String symbol   = readField(orderFrame);
        byte   side     = orderFrame.get();
        double qty      = orderFrame.getDouble();
        double price    = orderFrame.getDouble();

        writeField(out, "EXEC-001");
        writeField(out, "NYSE-ORD-001");
        writeField(out, clOrdId);
        writeField(out, symbol);
        out.put(side);
        out.putDouble(qty);
        out.putDouble(price);
    }

    static void writeField(ByteBuffer buf, String value) {
        byte[] bytes = value.getBytes(StandardCharsets.UTF_8);
        buf.put((byte) bytes.length);
        buf.put(bytes);
    }

    static String readField(ByteBuffer buf) {
        int len = buf.get() & 0xFF;
        byte[] bytes = new byte[len];
        buf.get(bytes);
        return new String(bytes, StandardCharsets.UTF_8);
    }

    // ── Aeron helpers ─────────────────────────────────────────────────────────

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

    static int findFreePort() throws Exception {
        try (java.net.ServerSocket s = new java.net.ServerSocket(0)) {
            return s.getLocalPort();
        }
    }

    // ── Result reporting ──────────────────────────────────────────────────────

    static void printResults() {
        if (latencies.isEmpty()) return;
        long[] sorted = latencies.stream().mapToLong(Long::longValue).sorted().toArray();
        int n = sorted.length;
        long sum = 0;
        for (long v : sorted) sum += v;

        System.out.println("  Round-trip latency (venue FIX NOS → venue FIX ExecRpt over TCP):");
        System.out.printf("    min:    %10.1f µs%n", sorted[0]                                  / 1_000.0);
        System.out.printf("    mean:   %10.1f µs%n", (sum / (double) n)                         / 1_000.0);
        System.out.printf("    p50:    %10.1f µs%n", sorted[n * 50  / 100]                      / 1_000.0);
        System.out.printf("    p90:    %10.1f µs%n", sorted[n * 90  / 100]                      / 1_000.0);
        System.out.printf("    p99:    %10.1f µs%n", sorted[n * 99  / 100]                      / 1_000.0);
        System.out.printf("    p99.9:  %10.1f µs%n", sorted[Math.min(n - 1, n * 999 / 1000)]    / 1_000.0);
        System.out.printf("    max:    %10.1f µs%n", sorted[n - 1]                              / 1_000.0);
        System.out.println();
        if (sessionElapsedNs > 0) {
            System.out.printf("  Throughput:                %.0f round-trips/sec%n",
                (double) n / (sessionElapsedNs / 1_000_000_000.0));
        }
    }
}
