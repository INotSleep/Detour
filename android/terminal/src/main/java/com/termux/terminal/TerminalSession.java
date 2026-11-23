package com.termux.terminal;

import java.util.UUID;

/**
 * A terminal session whose other end is the app (Detour's SSH core) rather than a local process:
 * the app feeds it the server's output with {@link #feed(byte[], int)} on the main thread, and
 * gets what is typed and the window's size through {@link Transport}.
 *
 * Changed by Detour from Termux's TerminalSession (Apache License 2.0), which ran a subprocess.
 */
public final class TerminalSession extends TerminalOutput {

    /** Where what is typed and the window's size go. */
    public interface Transport {
        void send(byte[] data);
        void resize(int columns, int rows);
    }

    public final String mHandle = UUID.randomUUID().toString();

    TerminalEmulator mEmulator;
    TerminalSessionClient mClient;
    private final Transport mTransport;
    private final Integer mTranscriptRows;
    private final byte[] mUtf8InputBuffer = new byte[5];
    /** Output that came before the view had a size. */
    private byte[] mPending = new byte[0];

    public TerminalSession(Integer transcriptRows, TerminalSessionClient client, Transport transport) {
        mTranscriptRows = transcriptRows;
        mClient = client;
        mTransport = transport;
    }

    public void updateTerminalSessionClient(TerminalSessionClient client) {
        mClient = client;
        if (mEmulator != null) mEmulator.updateTerminalSessionClient(client);
    }

    /** The view's new size: the emulator reflows, and the server is told. */
    public void updateSize(int columns, int rows) {
        if (mEmulator == null) {
            mEmulator = new TerminalEmulator(this, columns, rows, mTranscriptRows, mClient);
            if (mPending.length > 0) {
                mEmulator.append(mPending, mPending.length);
                mPending = new byte[0];
            }
        } else {
            mEmulator.resize(columns, rows);
        }
        mTransport.resize(columns, rows);
        notifyScreenUpdate();
    }

    /** The server's output. Main thread only. */
    public void feed(byte[] data, int length) {
        if (mEmulator == null) {
            byte[] joined = new byte[mPending.length + length];
            System.arraycopy(mPending, 0, joined, 0, mPending.length);
            System.arraycopy(data, 0, joined, mPending.length, length);
            mPending = joined;
            return;
        }
        mEmulator.append(data, length);
        notifyScreenUpdate();
    }

    public String getTitle() {
        return (mEmulator == null) ? null : mEmulator.getTitle();
    }

    /** What is typed, to the server. */
    @Override
    public void write(byte[] data, int offset, int count) {
        byte[] out = new byte[count];
        System.arraycopy(data, offset, out, 0, count);
        mTransport.send(out);
    }

    /** Write the Unicode code point to the terminal encoded in UTF-8. */
    public void writeCodePoint(boolean prependEscape, int codePoint) {
        if (codePoint > 1114111 || (codePoint >= 0xD800 && codePoint <= 0xDFFF)) {
            throw new IllegalArgumentException("Invalid code point: " + codePoint);
        }
        int p = 0;
        if (prependEscape) mUtf8InputBuffer[p++] = 27;
        if (codePoint <= 0b1111111) {
            mUtf8InputBuffer[p++] = (byte) codePoint;
        } else if (codePoint <= 0b11111111111) {
            mUtf8InputBuffer[p++] = (byte) (0b11000000 | (codePoint >> 6));
            mUtf8InputBuffer[p++] = (byte) (0b10000000 | (codePoint & 0b111111));
        } else if (codePoint <= 0b1111111111111111) {
            mUtf8InputBuffer[p++] = (byte) (0b11100000 | (codePoint >> 12));
            mUtf8InputBuffer[p++] = (byte) (0b10000000 | ((codePoint >> 6) & 0b111111));
            mUtf8InputBuffer[p++] = (byte) (0b10000000 | (codePoint & 0b111111));
        } else {
            mUtf8InputBuffer[p++] = (byte) (0b11110000 | (codePoint >> 18));
            mUtf8InputBuffer[p++] = (byte) (0b10000000 | ((codePoint >> 12) & 0b111111));
            mUtf8InputBuffer[p++] = (byte) (0b10000000 | ((codePoint >> 6) & 0b111111));
            mUtf8InputBuffer[p++] = (byte) (0b10000000 | (codePoint & 0b111111));
        }
        write(mUtf8InputBuffer, 0, p);
    }

    public TerminalEmulator getEmulator() {
        return mEmulator;
    }

    protected void notifyScreenUpdate() {
        if (mClient != null) mClient.onTextChanged(this);
    }

    public void reset() {
        if (mEmulator == null) return;
        mEmulator.reset();
        notifyScreenUpdate();
    }

    /** The session is always "running": its end is the app's to show. */
    public boolean isRunning() {
        return true;
    }

    @Override
    public void titleChanged(String oldTitle, String newTitle) {
        if (mClient != null) mClient.onTitleChanged(this);
    }

    @Override
    public void onCopyTextToClipboard(String text) {
        if (mClient != null) mClient.onCopyTextToClipboard(this, text);
    }

    @Override
    public void onPasteTextFromClipboard() {
        if (mClient != null) mClient.onPasteTextFromClipboard(this);
    }

    @Override
    public void onBell() {
        if (mClient != null) mClient.onBell(this);
    }

    @Override
    public void onColorsChanged() {
        if (mClient != null) mClient.onColorsChanged(this);
    }
}
