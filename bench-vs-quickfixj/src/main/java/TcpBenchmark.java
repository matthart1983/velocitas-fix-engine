import quickfix.*;
import quickfix.field.*;
import quickfix.fix44.NewOrderSingle;
import quickfix.fix44.ExecutionReport;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;

/**
 * QuickFIX/J TCP round-trip benchmark.
 *
 * Starts an acceptor and an initiator on localhost, exchanges N NewOrderSingle →
 * ExecutionReport round-trips, and reports latency statistics.
 */
public class TcpBenchmark {

    static final int DEFAULT_COUNT = 10_000;

    static int messageCount = DEFAULT_COUNT;
    static final CountDownLatch logonLatch = new CountDownLatch(1);
    static final CountDownLatch doneLatch = new CountDownLatch(1);

    // Latency tracking (in the initiator)
    static final List<Long> latencies = Collections.synchronizedList(new ArrayList<>());
    static final AtomicLong sendTime = new AtomicLong();
    static final AtomicInteger sentCount = new AtomicInteger(0);
    static long sessionStartNs;
    static long sessionElapsedNs;

    public static void main(String[] args) throws Exception {
        for (int i = 0; i < args.length; i++) {
            if ("--count".equals(args[i]) && i + 1 < args.length) {
                messageCount = Integer.parseInt(args[i + 1]);
            }
        }

        System.out.println("╔══════════════════════════════════════════════════════════════════╗");
        System.out.println("║         QuickFIX/J — TCP Round-Trip Benchmark                   ║");
        System.out.println("╚══════════════════════════════════════════════════════════════════╝");
        System.out.println();
        System.out.printf("  Messages: %d%n", messageCount);
        System.out.printf("  Flow:     Logon → %d×(NOS → ExecRpt) → Logout%n", messageCount);
        System.out.println();

        // Start acceptor
        SessionSettings acceptorSettings = new SessionSettings();
        Dictionary acceptorDefaults = new Dictionary();
        acceptorDefaults.setString("ConnectionType", "acceptor");
        acceptorDefaults.setString("SocketAcceptPort", "0"); // random port
        acceptorDefaults.setString("StartTime", "00:00:00");
        acceptorDefaults.setString("EndTime", "23:59:59");
        acceptorDefaults.setString("HeartBtInt", "30");
        acceptorDefaults.setString("ResetOnLogon", "Y");
        acceptorDefaults.setString("UseDataDictionary", "N");

        SessionID acceptorSessionId = new SessionID("FIX.4.4", "EXCHANGE", "TRADER");
        acceptorSettings.set(acceptorDefaults);
        acceptorSettings.set(acceptorSessionId, new Dictionary());

        BenchAcceptorApp acceptorApp = new BenchAcceptorApp();
        MessageStoreFactory storeFactory = new MemoryStoreFactory();
        LogFactory logFactory = new ScreenLogFactory(false, false, false);

        // Use SocketAcceptor — need to find a free port
        // QuickFIX/J doesn't support port 0, so find one manually
        int port = findFreePort();
        acceptorDefaults.setString("SocketAcceptPort", String.valueOf(port));
        acceptorSettings.set(acceptorDefaults);

        SocketAcceptor acceptor = new SocketAcceptor(
            acceptorApp, storeFactory, acceptorSettings, logFactory, new DefaultMessageFactory()
        );
        acceptor.start();

        // Start initiator
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

        SessionID initiatorSessionId = new SessionID("FIX.4.4", "TRADER", "EXCHANGE");
        initiatorSettings.set(initiatorDefaults);
        initiatorSettings.set(initiatorSessionId, new Dictionary());

        BenchInitiatorApp initiatorApp = new BenchInitiatorApp(initiatorSessionId);

        SocketInitiator initiator = new SocketInitiator(
            initiatorApp, storeFactory, initiatorSettings, logFactory, new DefaultMessageFactory()
        );
        initiator.start();

        // Wait for completion
        doneLatch.await(120, TimeUnit.SECONDS);

        initiator.stop();
        acceptor.stop();

        // Report
        reportLatencies();

        double totalMsgs = messageCount * 2.0;
        double elapsedSec = sessionElapsedNs / 1_000_000_000.0;
        System.out.println();
        System.out.printf("  Session total:             %.1f ms%n", sessionElapsedNs / 1_000_000.0);
        System.out.printf("  Application messages:      %d (%d round-trips)%n", (int) totalMsgs, messageCount);
        System.out.printf("  Throughput:                %.0f msgs/sec%n", totalMsgs / elapsedSec);
        System.out.printf("  Round-trips/sec:           %.0f%n", messageCount / elapsedSec);
        System.out.println();
    }

    static int findFreePort() throws Exception {
        try (java.net.ServerSocket s = new java.net.ServerSocket(0)) {
            return s.getLocalPort();
        }
    }

    static void reportLatencies() {
        if (latencies.isEmpty()) return;
        long[] sorted = latencies.stream().mapToLong(Long::longValue).sorted().toArray();
        int len = sorted.length;
        long sum = 0;
        for (long v : sorted) sum += v;
        long mean = sum / len;

        System.out.println("  Round-trip latency (NOS → ExecRpt):");
        System.out.printf("    min:    %10s µs%n", formatUs(sorted[0]));
        System.out.printf("    mean:   %10s µs%n", formatUs(mean));
        System.out.printf("    p50:    %10s µs%n", formatUs(sorted[len * 50 / 100]));
        System.out.printf("    p90:    %10s µs%n", formatUs(sorted[len * 90 / 100]));
        System.out.printf("    p99:    %10s µs%n", formatUs(sorted[len * 99 / 100]));
        System.out.printf("    p99.9:  %10s µs%n", formatUs(sorted[Math.min(len - 1, len * 999 / 1000)]));
        System.out.printf("    max:    %10s µs%n", formatUs(sorted[len - 1]));
    }

    static String formatUs(long nanos) {
        return String.format("%.1f", nanos / 1000.0);
    }

    // ─────────────────────────────────────────────────────────────
    // Acceptor App — responds to every NOS with an ExecutionReport
    // ─────────────────────────────────────────────────────────────

    static class BenchAcceptorApp extends MessageCracker implements Application {
        @Override public void onCreate(SessionID sessionId) {}
        @Override public void onLogon(SessionID sessionId) {}
        @Override public void onLogout(SessionID sessionId) {}
        @Override public void toAdmin(Message message, SessionID sessionId) {}
        @Override public void fromAdmin(Message message, SessionID sessionId) {}
        @Override public void toApp(Message message, SessionID sessionId) {}

        @Override
        public void fromApp(Message message, SessionID sessionId) throws FieldNotFound, UnsupportedMessageType, IncorrectTagValue {
            String msgType = message.getHeader().getString(MsgType.FIELD);
            if ("D".equals(msgType)) {
                try {
                    String clOrdId = message.getString(ClOrdID.FIELD);
                    String symbol = message.getString(Symbol.FIELD);
                    double qty = message.getDouble(OrderQty.FIELD);

                    ExecutionReport report = new ExecutionReport(
                        new OrderID("ORD-001"),
                        new ExecID("EXEC-001"),
                        new ExecType(ExecType.FILL),
                        new OrdStatus(OrdStatus.FILLED),
                        new Side(Side.BUY),
                        new LeavesQty(0),
                        new CumQty(qty),
                        new AvgPx(100.00)
                    );
                    report.set(new ClOrdID(clOrdId));
                    report.set(new Symbol(symbol));
                    report.set(new OrderQty(qty));
                    report.set(new LastPx(100.00));
                    report.set(new LastQty(qty));

                    Session.sendToTarget(report, sessionId);
                } catch (SessionNotFound e) {
                    e.printStackTrace();
                }
            }
        }
    }

    // ─────────────────────────────────────────────────────────────
    // Initiator App — sends N NOS, measures round-trip latency
    // ─────────────────────────────────────────────────────────────

    static class BenchInitiatorApp extends MessageCracker implements Application {
        private final SessionID sessionId;

        BenchInitiatorApp(SessionID sessionId) {
            this.sessionId = sessionId;
        }

        @Override public void onCreate(SessionID sessionId) {}

        @Override
        public void onLogon(SessionID sessionId) {
            sessionStartNs = System.nanoTime();
            // Send first NOS
            sendNos();
        }

        @Override public void onLogout(SessionID sessionId) {}
        @Override public void toAdmin(Message message, SessionID sessionId) {}
        @Override public void fromAdmin(Message message, SessionID sessionId) {}
        @Override public void toApp(Message message, SessionID sessionId) {}

        @Override
        public void fromApp(Message message, SessionID sessionId) throws FieldNotFound, UnsupportedMessageType, IncorrectTagValue {
            String msgType = message.getHeader().getString(MsgType.FIELD);
            if ("8".equals(msgType)) {
                long rtt = System.nanoTime() - sendTime.get();
                latencies.add(rtt);

                if (sentCount.get() < messageCount) {
                    sendNos();
                } else {
                    sessionElapsedNs = System.nanoTime() - sessionStartNs;
                    doneLatch.countDown();
                }
            }
        }

        private void sendNos() {
            try {
                int seq = sentCount.incrementAndGet();
                NewOrderSingle nos = new NewOrderSingle(
                    new ClOrdID("ORD-" + String.format("%08d", seq)),
                    new Side(Side.BUY),
                    new TransactTime(),
                    new OrdType(OrdType.LIMIT)
                );
                nos.set(new Symbol("AAPL"));
                nos.set(new OrderQty(100));
                nos.set(new Price(100.00));

                sendTime.set(System.nanoTime());
                Session.sendToTarget(nos, sessionId);
            } catch (SessionNotFound e) {
                e.printStackTrace();
            }
        }
    }
}
