package dev.detour.android.core

import android.content.Context
import android.os.Handler
import android.os.Looper
import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener
import uniffi.detour_mobile.CoreException
import uniffi.detour_mobile.CoreListener
import uniffi.detour_mobile.DetourCore
import java.io.File
import java.util.concurrent.atomic.AtomicLong

/** A call the core refused, by its kind (`locked`, `needSecret`, `hostKeyUnknown`, ...). */
class CoreFailure(val kind: String, val fields: Map<String, Any?>) : Exception(kind) {
    operator fun get(key: String): String? = fields[key] as? String

    companion object {
        @Suppress("UNCHECKED_CAST")
        fun from(json: String): CoreFailure {
            val o = runCatching { Json.parse(json) as? Map<String, Any?> }.getOrNull() ?: emptyMap()
            return CoreFailure(o["kind"] as? String ?: "internal", o)
        }
    }
}

/** JSON as Kotlin maps, lists, strings, numbers and booleans. */
object Json {
    fun parse(text: String): Any? = unwrap(JSONTokener(text).nextValue())

    fun unwrap(v: Any?): Any? = when (v) {
        is JSONObject -> v.keys().asSequence().associateWith { unwrap(v.opt(it)) }
        is JSONArray -> (0 until v.length()).map { unwrap(v.opt(it)) }
        JSONObject.NULL -> null
        else -> v
    }

    fun wrap(v: Any?): Any? = when (v) {
        null -> JSONObject.NULL
        is Map<*, *> -> JSONObject().apply { v.forEach { (k, x) -> if (x != null) put(k as String, wrap(x)) } }
        is List<*> -> JSONArray().apply { v.forEach { put(wrap(it)) } }
        is Array<*> -> wrap(v.toList())
        else -> v
    }

    fun stringify(v: Any?): String = when (val w = wrap(v)) {
        is JSONObject, is JSONArray -> w.toString()
        is String -> JSONObject.quote(w)
        else -> w.toString()
    }
}

@Suppress("UNCHECKED_CAST")
fun Any?.obj(): Map<String, Any?> = this as? Map<String, Any?> ?: emptyMap()
@Suppress("UNCHECKED_CAST")
fun Any?.list(): List<Any?> = this as? List<Any?> ?: emptyList()
fun Map<String, Any?>.str(key: String): String? = this[key] as? String
fun Map<String, Any?>.int(key: String): Int? = (this[key] as? Number)?.toInt()
fun Map<String, Any?>.double(key: String): Double? = (this[key] as? Number)?.toDouble()
fun Map<String, Any?>.bool(key: String): Boolean? = this[key] as? Boolean

/**
 * Detour's core, inside the app. Calls go to it as JSON; what it sends on its own (events,
 * terminal output, a session's end) reaches whoever listens here, on the main thread.
 */
object Core {
    private lateinit var core: DetourCore
    private val listener = Listener()
    private val next = AtomicLong(1)

    fun start(context: Context) {
        if (::core.isInitialized) return
        core = DetourCore(File(context.filesDir, "Detour").path, listener)
    }

    /** Calls the core; the result is its JSON as maps and lists. Null values are left out. */
    suspend fun call(method: String, params: Map<String, Any?> = emptyMap()): Any? {
        val body = Json.stringify(params.filterValues { it != null })
        val out = try {
            core.call(method, body)
        } catch (e: CoreException.Failed) {
            throw CoreFailure.from(e.json)
        }
        return if (out.isBlank()) null else Json.parse(out)
    }

    /** A stream id for a session's output. */
    fun newStream(): ULong = next.incrementAndGet().toULong()

    fun watch(stream: ULong, data: (ByteArray) -> Unit, end: (Map<String, Any?>) -> Unit) = listener.watch(stream, data, end)
    fun unwatch(stream: ULong) = listener.unwatch(stream)
    /** Progress of a transfer (`{ done, total }`) until `unwatch`. */
    fun watchProgress(stream: ULong, progress: (Double, Double) -> Unit) = listener.watchProgress(stream, progress)
    /** Events of the core (`sessions.changed`, `locked`, `data.changed`...). */
    fun onEvent(handler: (String, Map<String, Any?>) -> Unit) = listener.onEvent(handler)
    fun stop() = core.stop()
}

/** Receives from the core on its threads, hands on to the main one. */
private class Listener : CoreListener {
    private class Watch(val data: (ByteArray) -> Unit, val end: (Map<String, Any?>) -> Unit)

    private val main = Handler(Looper.getMainLooper())
    private val streams = HashMap<ULong, Watch>()
    private val progress = HashMap<ULong, (Double, Double) -> Unit>()
    private val handlers = ArrayList<(String, Map<String, Any?>) -> Unit>()

    @Synchronized fun watch(stream: ULong, data: (ByteArray) -> Unit, end: (Map<String, Any?>) -> Unit) { streams[stream] = Watch(data, end) }
    @Synchronized fun watchProgress(stream: ULong, p: (Double, Double) -> Unit) { progress[stream] = p }
    @Synchronized fun unwatch(stream: ULong) { streams.remove(stream); progress.remove(stream) }
    @Synchronized fun onEvent(handler: (String, Map<String, Any?>) -> Unit) { handlers.add(handler) }

    override fun onEvent(name: String, data: String) {
        val obj = runCatching { Json.parse(data).obj() }.getOrDefault(emptyMap())
        val hs = synchronized(this) { handlers.toList() }
        main.post { hs.forEach { it(name, obj) } }
    }

    override fun onData(stream: ULong, bytes: ByteArray) {
        val w = synchronized(this) { streams[stream] } ?: return
        main.post { w.data(bytes) }
    }

    override fun onStreamEnd(stream: ULong, data: String) {
        val obj = runCatching { Json.parse(data).obj() }.getOrDefault(emptyMap())
        synchronized(this) {
            // A transfer's progress comes the same way as a session's end, and keeps coming.
            progress[stream]?.let { p ->
                val done = obj.double("done") ?: 0.0
                val total = obj.double("total") ?: 0.0
                main.post { p(done, total) }
                return
            }
        }
        val w = synchronized(this) { streams.remove(stream) } ?: return
        main.post { w.end(obj) }
    }
}
