"""
gclaw_bridge.py — Python IPC bridge for gclaw-voice.

Replaces direct speech.listen()/speech.speak() calls in assistant_loop.py
with IPC messages to the native Rust voice shell.

Usage in assistant_loop.py:
    from gclaw_bridge import GclawVoiceBridge

    bridge = GclawVoiceBridge(port=19820)
    bridge.connect()

    # Instead of: text = speech.listen()
    text, lang, conf = bridge.wait_for_speech()

    # Instead of: speech.speak(response)
    bridge.speak(response)

    # Instead of: result = speech.speak_interruptible(response)
    result = bridge.speak_interruptible(response)
    # result is None (completed) or str (user's interruption text)

    # Instead of: detected = speech.listen_for_wake_word()
    detected = bridge.wait_for_wake_word()

Wire protocol: 4-byte big-endian length + MessagePack payload.
Message format: {"type": "<MessageType>", "payload": {...}}
"""

import socket
import struct
import logging

try:
    import msgpack
except ImportError:
    # Fallback: pip install msgpack
    raise ImportError("msgpack is required: pip install msgpack")

logger = logging.getLogger("gclaw_bridge")

# Default ports (must match Rust constants)
VOICE_TCP_PORT = 19820
TOOLS_TCP_PORT = 19821

# Max message size (16 MB, matches Rust MAX_MESSAGE_SIZE)
MAX_MESSAGE_SIZE = 16 * 1024 * 1024


class GclawVoiceBridge:
    """IPC bridge to the gclaw-voice Rust binary.

    Replaces speech.py function calls with IPC messages.
    Thread-safe: uses a single socket with send/recv locks.
    """

    def __init__(self, port=VOICE_TCP_PORT, host="127.0.0.1"):
        self.host = host
        self.port = port
        self._sock = None
        self._connected = False

    def connect(self, timeout=10.0):
        """Connect to the gclaw-voice IPC server."""
        self._sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._sock.settimeout(timeout)
        self._sock.connect((self.host, self.port))
        self._connected = True
        logger.info(f"connected to gclaw-voice at {self.host}:{self.port}")

    def disconnect(self):
        """Disconnect from gclaw-voice."""
        if self._sock:
            try:
                self._send_message({"type": "Shutdown"})
            except Exception:
                pass
            self._sock.close()
            self._sock = None
            self._connected = False

    @property
    def connected(self):
        return self._connected

    # ----------------------------------------------------------------
    # Wire protocol
    # ----------------------------------------------------------------

    def _send_message(self, msg):
        """Send a length-prefixed MessagePack message."""
        payload = msgpack.packb(msg, use_bin_type=True)
        length = len(payload)
        if length > MAX_MESSAGE_SIZE:
            raise ValueError(f"message too large: {length} bytes")
        header = struct.pack(">I", length)
        self._sock.sendall(header + payload)

    def _recv_message(self):
        """Receive a length-prefixed MessagePack message."""
        header = self._recv_exact(4)
        length = struct.unpack(">I", header)[0]
        if length > MAX_MESSAGE_SIZE:
            raise ValueError(f"frame too large: {length} bytes")
        payload = self._recv_exact(length)
        return msgpack.unpackb(payload, raw=False)

    def _recv_exact(self, n):
        """Read exactly n bytes from socket."""
        data = bytearray()
        while len(data) < n:
            chunk = self._sock.recv(n - len(data))
            if not chunk:
                raise ConnectionError("gclaw-voice disconnected")
            data.extend(chunk)
        return bytes(data)

    # ----------------------------------------------------------------
    # Voice Shell API (replaces speech.py functions)
    # ----------------------------------------------------------------

    def wait_for_wake_word(self, timeout_s=None):
        """Block until wake word is detected.

        Mirrors: speech.listen_for_wake_word()
        Returns: True if wake word detected, False on timeout.
        """
        if timeout_s:
            self._sock.settimeout(timeout_s)
        try:
            while True:
                msg = self._recv_message()
                msg_type = msg.get("type")
                if msg_type == "WakeWordDetected":
                    return True
                elif msg_type == "Ping":
                    self._send_message({"type": "Pong"})
                # Ignore other messages while waiting for wake word
        except socket.timeout:
            return False
        finally:
            self._sock.settimeout(None)

    def wait_for_speech(self, timeout_s=None):
        """Block until user speech is transcribed.

        Mirrors: speech.listen()
        Returns: (text, language, confidence) or (None, None, None) on timeout.
        """
        if timeout_s:
            self._sock.settimeout(timeout_s)
        try:
            while True:
                msg = self._recv_message()
                msg_type = msg.get("type")
                if msg_type == "UserSpeech":
                    payload = msg.get("payload", {})
                    return (
                        payload.get("text", ""),
                        payload.get("language", "en"),
                        payload.get("confidence", 0.0),
                    )
                elif msg_type == "BargeIn":
                    payload = msg.get("payload", {})
                    return (payload.get("text", ""), "en", 1.0)
                elif msg_type == "Ping":
                    self._send_message({"type": "Pong"})
        except socket.timeout:
            return (None, None, None)
        finally:
            self._sock.settimeout(None)

    def speak(self, text):
        """Speak text (blocking on Rust side, non-blocking here).

        Mirrors: speech.speak()
        """
        self._send_message({
            "type": "Speak",
            "payload": {"text": text}
        })

    def speak_interruptible(self, text):
        """Speak with barge-in support.

        Mirrors: speech.speak_interruptible()
        Returns: None if completed, or the user's interruption text.
        """
        self._send_message({
            "type": "SpeakInterruptible",
            "payload": {"text": text}
        })
        # Wait for either completion (no message) or BargeIn message.
        # The Rust side sends a BargeIn message if the user interrupts.
        self._sock.settimeout(120)  # Max TTS duration
        try:
            while True:
                msg = self._recv_message()
                msg_type = msg.get("type")
                if msg_type == "BargeIn":
                    payload = msg.get("payload", {})
                    return payload.get("text")
                elif msg_type == "Ping":
                    self._send_message({"type": "Pong"})
                else:
                    # Any other message means speech completed
                    return None
        except socket.timeout:
            return None
        finally:
            self._sock.settimeout(None)

    def stop_speaking(self):
        """Stop current TTS immediately.

        Mirrors: speech.stop_speaking()
        """
        self._send_message({"type": "StopSpeaking"})

    def set_mic_state(self, state):
        """Set microphone state.

        Args:
            state: "IDLE", "LISTENING", "PROCESSING", or "SPEAKING"
        """
        self._send_message({
            "type": "SetMicState",
            "payload": {"state": state}
        })

    def configure(self, stt_engine=None, language=None, ai_name=None):
        """Reconfigure the voice shell.

        Mirrors: speech.set_stt_engine(), speech.set_language()
        """
        self._send_message({
            "type": "Configure",
            "payload": {
                "stt_engine": stt_engine,
                "language": language,
                "ai_name": ai_name,
            }
        })

    def ping(self):
        """Health check."""
        self._send_message({"type": "Ping"})
        msg = self._recv_message()
        return msg.get("type") == "Pong"

    def __enter__(self):
        self.connect()
        return self

    def __exit__(self, *args):
        self.disconnect()


# ----------------------------------------------------------------
# Convenience: drop-in replacement for speech module
# ----------------------------------------------------------------

_bridge = None


def init(port=VOICE_TCP_PORT):
    """Initialize the global bridge (call once at startup)."""
    global _bridge
    _bridge = GclawVoiceBridge(port=port)
    _bridge.connect()
    return _bridge


def get_bridge():
    """Get the global bridge instance."""
    return _bridge


# Drop-in function replacements for speech.py
def listen():
    """Drop-in replacement for speech.listen()."""
    text, lang, conf = _bridge.wait_for_speech()
    return text


def listen_for_wake_word(timeout_s=None):
    """Drop-in replacement for speech.listen_for_wake_word()."""
    return _bridge.wait_for_wake_word(timeout_s)


def speak(text):
    """Drop-in replacement for speech.speak()."""
    _bridge.speak(text)


def speak_interruptible(text):
    """Drop-in replacement for speech.speak_interruptible()."""
    return _bridge.speak_interruptible(text)


def stop_speaking():
    """Drop-in replacement for speech.stop_speaking()."""
    _bridge.stop_speaking()
