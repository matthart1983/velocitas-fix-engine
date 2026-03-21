import quickfix.*;
import quickfix.field.*;
import quickfix.fix44.NewOrderSingle;
import quickfix.fix44.ExecutionReport;

/**
 * QuickFIX/J benchmark — parse and serialize FIX messages.
 * Measures latency and throughput for direct comparison with Velocitas.
 */
public class QuickFixJBenchmark {

    static final int WARMUP_ITERATIONS = 100_000;
    static final int BENCH_ITERATIONS  = 1_000_000;

    // A realistic NewOrderSingle as raw FIX
    static final String NOS_RAW =
        "8=FIX.4.4\u00019=148\u000135=D\u000149=BANK_OMS\u000156=NYSE\u000134=42\u0001" +
        "52=20260321-10:00:00.123\u000111=ORD-2026032100001\u000155=AAPL\u000154=1\u0001" +
        "60=20260321-10:00:00.123\u000138=10000\u000140=2\u000144=178.55\u000110=000\u0001";

    // A realistic ExecutionReport as raw FIX
    static final String EXECRPT_RAW =
        "8=FIX.4.4\u00019=220\u000135=8\u000149=NYSE\u000156=BANK_OMS\u000134=100\u0001" +
        "52=20260321-10:00:00.456\u000137=NYSE-ORD-001\u000117=NYSE-EXEC-001\u0001" +
        "11=ORD-2026032100001\u000155=AAPL\u000154=1\u000138=10000\u000132=5000\u0001" +
        "31=178.55\u000114=5000\u0001151=5000\u00016=178.55\u0001150=F\u000139=1\u000110=000\u0001";

    // A Heartbeat
    static final String HB_RAW =
        "8=FIX.4.4\u00019=56\u000135=0\u000149=SENDER\u000156=TARGET\u000134=1\u0001" +
        "52=20260321-10:00:00.000\u000110=000\u0001";

    static final DataDictionary DATA_DICT;
    static {
        try {
            DATA_DICT = new DataDictionary("FIX44.xml");
            DATA_DICT.setCheckFieldsOutOfOrder(false);
            DATA_DICT.setCheckUnorderedGroupFields(false);
        } catch (ConfigError e) {
            throw new RuntimeException(e);
        }
    }

    public static void main(String[] args) throws Exception {
        System.out.println("╔══════════════════════════════════════════════════════════════════╗");
        System.out.println("║           QuickFIX/J Benchmark  (Java " + System.getProperty("java.version") + ")              ║");
        System.out.println("╚══════════════════════════════════════════════════════════════════╝");
        System.out.println();

        benchParse("Heartbeat", HB_RAW);
        benchParse("NewOrderSingle", NOS_RAW);
        benchParse("ExecutionReport", EXECRPT_RAW);

        System.out.println();
        benchSerialize();

        System.out.println();
        benchParseThoughput();
    }

    /** Compute a valid FIX checksum and replace the trailing 10=000\x01 */
    static String fixChecksum(String raw) {
        // Checksum covers everything up to (not including) "10="
        int idx = raw.lastIndexOf("10=");
        String body = raw.substring(0, idx);
        int sum = 0;
        for (int i = 0; i < body.length(); i++) {
            sum += body.charAt(i);
        }
        sum = sum % 256;
        return body + String.format("10=%03d\u0001", sum);
    }

    /** Parse latency benchmark — parse raw FIX string into Message object. */
    static void benchParse(String label, String raw) throws Exception {
        raw = fixChecksum(raw);

        // Warmup
        for (int i = 0; i < WARMUP_ITERATIONS; i++) {
            Message msg = new Message(raw, DATA_DICT, true);
        }

        // Measure
        long start = System.nanoTime();
        for (int i = 0; i < BENCH_ITERATIONS; i++) {
            Message msg = new Message(raw, DATA_DICT, true);
        }
        long elapsed = System.nanoTime() - start;
        double nsPerOp = (double) elapsed / BENCH_ITERATIONS;
        System.out.printf("  Parse %-20s  %,.0f ns/msg  (%d bytes)%n", label, nsPerOp, raw.length());
    }

    /** Serialize benchmark — build a NewOrderSingle and call toString(). */
    static void benchSerialize() throws Exception {
        // Warmup
        for (int i = 0; i < WARMUP_ITERATIONS; i++) {
            buildNos(i).toString();
        }

        // Measure
        long start = System.nanoTime();
        for (int i = 0; i < BENCH_ITERATIONS; i++) {
            buildNos(i).toString();
        }
        long elapsed = System.nanoTime() - start;
        double nsPerOp = (double) elapsed / BENCH_ITERATIONS;
        System.out.printf("  Serialize NOS              %,.0f ns/msg%n", nsPerOp);
    }

    /** Throughput benchmark — parse 1M messages, report msgs/sec. */
    static void benchParseThoughput() throws Exception {
        int count = BENCH_ITERATIONS;
        String raw = fixChecksum(NOS_RAW);

        // Warmup
        for (int i = 0; i < WARMUP_ITERATIONS; i++) {
            new Message(raw, DATA_DICT, true);
        }

        long start = System.nanoTime();
        for (int i = 0; i < count; i++) {
            new Message(raw, DATA_DICT, true);
        }
        long elapsed = System.nanoTime() - start;
        double msgsPerSec = (double) count / elapsed * 1_000_000_000.0;
        System.out.printf("  Throughput (NOS parse)      %,.0f msgs/sec  (%,d messages)%n", msgsPerSec, count);
    }

    static NewOrderSingle buildNos(int seq) {
        NewOrderSingle nos = new NewOrderSingle(
            new ClOrdID("ORD-" + seq),
            new Side(Side.BUY),
            new TransactTime(),
            new OrdType(OrdType.LIMIT)
        );
        nos.getHeader().setField(new BeginString("FIX.4.4"));
        nos.getHeader().setField(new SenderCompID("BANK_OMS"));
        nos.getHeader().setField(new TargetCompID("NYSE"));
        nos.getHeader().setField(new MsgSeqNum(seq));
        nos.set(new Symbol("AAPL"));
        nos.set(new OrderQty(10000));
        nos.set(new Price(178.55));
        return nos;
    }
}
