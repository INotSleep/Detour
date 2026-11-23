package dev.detour.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import dev.detour.android.core.t

/**
 * Keeps the app running in the background while sessions or tunnels are open, with a
 * notification that says so (Android stops a background app's connections otherwise).
 */
class KeepAliveService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val text = summary()
        if (text == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, t("Connections"), NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val n = Notification.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_upload_done)
            .setContentTitle("Detour")
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= 34) startForeground(ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        else startForeground(ID, n)
        return START_NOT_STICKY
    }

    companion object {
        private const val CHANNEL = "connections"
        private const val ID = 1
        lateinit var context: Context

        /** What is open: "2 sessions, 1 tunnel", or nothing. */
        private fun summary(): String? {
            val parts = buildList {
                val s = AppModel.sessions.size
                if (s > 0) add(t("Sessions: %@", s))
                val tunnels = Tunnels.running
                if (tunnels > 0) add(t("Tunnels: %@", tunnels))
            }
            return parts.takeIf { it.isNotEmpty() }?.joinToString(", ")
        }

        /** Asks once for notifications (Android 13 and later): the one saying what runs. */
        var askNotifications: (() -> Unit)? = null
        private var asked = false

        /** Starts, updates or stops the service for what is open now. */
        fun update() {
            if (!::context.isInitialized) return
            val intent = Intent(context, KeepAliveService::class.java)
            if (summary() == null) {
                context.stopService(intent)
                return
            }
            if (!asked) {
                asked = true
                askNotifications?.invoke()
            }
            runCatching { context.startForegroundService(intent) }
        }
    }
}
