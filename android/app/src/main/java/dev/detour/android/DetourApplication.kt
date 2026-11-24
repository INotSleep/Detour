package dev.detour.android

import android.app.Application
import dev.detour.android.core.Core
import dev.detour.android.core.L10n
import dev.detour.android.core.Native

class DetourApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        L10n.load(this)
        dev.detour.android.core.Haptics.load(this)
        dev.detour.android.terminal.Clipboard.context = this
        KeepAliveService.context = this
        dev.detour.android.keys.KeyBarStore.load(this)
        dev.detour.android.keys.KeysKeyboardStore.load(this)
        Native.init(this)
        Core.start(this)
    }
}
