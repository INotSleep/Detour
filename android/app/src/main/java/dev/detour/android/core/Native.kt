package dev.detour.android.core

import android.content.Context

/** What the core needs from Android before it starts: the system's TLS certificate checks. */
object Native {
    init {
        System.loadLibrary("detour_mobile")
    }

    @JvmStatic
    external fun init(context: Context)
}
