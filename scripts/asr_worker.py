"""常驻转写进程：加载一次模型，之后按行收 WAV 路径、回一行 JSON。

为什么不直接用 `whisper file.wav`：CLI 每次都要重新 import torch、加载模型
（实测冷启动 6～8 秒），而语音输入是「松手就要看见字」的场景，等不起。
常驻进程只在第一次启动时付这笔钱，之后每段音频只剩推理时间。

引擎优先 faster-whisper（CTranslate2，同模型快 4～8 倍，自带 VAD），
装不上时回退 openai-whisper，保证功能不会因为一个包缺失而整个不可用。

协议（只用 stdout，一行一个 JSON）：
    输入：WAV 的绝对路径；`__quit__` 表示退出。
    输出：{"ready": true, "model": "...", "engine": "...", "device": "..."}
          {"text": "...", "quiet": false} / {"error": "..."}

模型的进度条与警告都走 stderr，不会污染协议流。
"""
import array
import json
import os
import sys
import wave

# 默认用 turbo：large-v3 的蒸馏版，中文比 small 好一大截，显存只要一半左右。
DEFAULT_MODEL = os.environ.get("WORKBUDDY_PET_ASR_MODEL", "large-v3-turbo")

# 引导它吐标点的提示词。副作用是「没有语音内容时它会把这句原样吐回来」，
# 所以下面要按它做一次回声过滤——否则误触录音会变成往对话框里塞这一行。
GUIDE = "以下是普通话的句子，请使用简体中文并加标点。"
PROMPT = GUIDE

# 注意：**不要**用 faster-whisper 的 `hotwords`。实测它会把词表硬塞进结果
# （「只保留主分支」被改成「只保留，主分支」，还多出「云端」），因为
# hotwords 是解码偏置而不是纠错。词表只作为 initial_prompt 的弱提示，
# 或者干脆不加——上下文比词表可靠。

# 半角标点在中文字后面统一成全角：whisper 的中文输出默认用 `,`，
# 粘进对话里看着像没断句。只在「前一个字符是汉字」时替换，不碰数字和英文。
_PUNCT = {",": "，", ".": "。", "?": "？", "!": "！", ":": "：", ";": "；"}


def _is_cjk(ch):
    return "\u4e00" <= ch <= "\u9fff"


def normalize_punct(text):
    out = []
    for i, ch in enumerate(text):
        prev = text[i - 1] if i else ""
        out.append(_PUNCT.get(ch, ch) if _is_cjk(prev) else ch)
    return "".join(out)


def emit(payload):
    print(json.dumps(payload, ensure_ascii=False), flush=True)


def audio_level(path):
    """返回 (peak, rms)，只统计前 30 秒。

    设备麦克风是远场的，人一后退电平就掉一个数量级；先量一下，识别为空时
    才能分清「环境太吵/声音太小」和「模型没听懂」。
    """
    try:
        with wave.open(path, "rb") as handle:
            frames = min(handle.getnframes(), handle.getframerate() * 30)
            raw = handle.readframes(frames)
    except Exception:
        return 0, 0.0
    samples = array.array("h")
    samples.frombytes(raw[: len(raw) // 2 * 2])
    if not samples:
        return 0, 0.0
    peak = max(abs(s) for s in samples)
    rms = (sum(float(s) * s for s in samples) / len(samples)) ** 0.5
    return peak, rms


def load_wav(path):
    """把 WAV 读成 16 kHz 单声道 float32。

    不交给 faster-whisper 内部的 PyAV 解码：那边要求特定 av 版本，实测
    av 19 上 `metadata_errors` 参数已经没了，直接 TypeError。设备录音就是
    标准 PCM16 WAV，自己读既稳又快（也顺手拿到电平）。
    返回 (samples, rate)；读不了就返回 (None, 0) 让调用方退回路径输入。
    """
    try:
        import numpy as np

        with wave.open(path, "rb") as handle:
            rate = handle.getframerate()
            channels = handle.getnchannels()
            width = handle.getsampwidth()
            raw = handle.readframes(handle.getnframes())
        if width != 2:
            return None, rate
        audio = np.frombuffer(raw, dtype="<i2").astype("float32") / 32768.0
        if channels > 1:
            audio = audio.reshape(-1, channels).mean(axis=1)
        return audio, rate
    except Exception:
        return None, 0


def echoed_prompt(text):
    """识别结果是不是把引导词原样吐回来了。"""
    def norm(s):
        return "".join(ch for ch in s if ch.isalnum())

    return bool(norm(text)) and norm(text) in norm(PROMPT)


def add_cuda_dlls():
    """把 torch 自带的 cuBLAS / cuDNN 目录挂进 DLL 搜索路径。

    CTranslate2 不会自动找 torch 的库：Windows 上缺 `cublas64_12.dll` 会直接报
    「Library cublas64_12.dll is not found」，然后整个 GPU 路径都不可用。
    这些 DLL 随 torch 装着，挂上目录即可，不必再单独装 nvidia-* 包。
    """
    import os
    import site
    import sysconfig

    candidates = []
    try:
        candidates.append(os.path.join(site.getusersitepackages(), "torch", "lib"))
    except Exception:  # noqa: BLE001
        pass
    try:
        candidates.append(os.path.join(sysconfig.get_paths()["purelib"], "torch", "lib"))
    except Exception:  # noqa: BLE001
        pass
    for path in candidates:
        if not os.path.isdir(path):
            continue
        try:
            os.add_dll_directory(path)
        except Exception:  # noqa: BLE001
            pass
        os.environ["PATH"] = path + os.pathsep + os.environ.get("PATH", "")


class Engine:
    """统一 faster-whisper 与 openai-whisper 两套 API。"""

    def __init__(self, name):
        self.name = name
        self.kind = ""
        self.device = ""
        self.model = None
        self._load()

    def _load(self):
        add_cuda_dlls()
        try:
            from faster_whisper import WhisperModel

            # GPU 优先，但 cuDNN 版本不匹配是常事：失败就退 CPU int8，
            # 10 秒以内的短句在 CPU 上也能 1 秒内出字。
            for dev, compute in (("cuda", "float16"), ("cpu", "int8")):
                try:
                    self.model = WhisperModel(self.name, device=dev, compute_type=compute)
                    self.kind, self.device = "faster-whisper", f"{dev}/{compute}"
                    return
                except Exception as exc:  # noqa: BLE001
                    print(f"[asr] {dev}/{compute} 不可用：{exc}", file=sys.stderr, flush=True)
        except Exception as exc:  # noqa: BLE001
            print(f"[asr] faster-whisper 不可用：{exc}", file=sys.stderr, flush=True)

        import whisper

        self.model = whisper.load_model(self.name)
        self.kind, self.device = "whisper", "torch"

    def transcribe(self, path):
        # 设备录音是 16 kHz 单声道，直接喂数组，绕开 PyAV 的版本坑。
        audio, rate = load_wav(path)
        source = audio if audio is not None and rate == 16000 else path
        if self.kind == "faster-whisper":
            segments, _info = self.model.transcribe(
                source,
                language="zh",
                beam_size=5,
                # VAD 把首尾静音切掉：设备录音前后常有 0.5 秒环境噪声，
                # 不切掉 whisper 容易把它「听」成一句话。
                vad_filter=True,
                vad_parameters={"min_silence_duration_ms": 300},
                initial_prompt=GUIDE,
                condition_on_previous_text=False,
            )
            parts = []
            speech = False
            for segment in segments:
                parts.append(segment.text)
                if (segment.no_speech_prob or 0.0) <= 0.5:
                    speech = True
            text = "".join(parts).strip()
            if text and not speech:
                return ""  # 整段都被判成非语音：环境噪声，不是人话
        else:
            result = self.model.transcribe(
                source,
                language="zh",
                initial_prompt=PROMPT,
                condition_on_previous_text=False,
                verbose=None,
            )
            text = (result.get("text") or "").strip()
            segments = result.get("segments") or []
            if segments and all(s.get("no_speech_prob", 0.0) > 0.5 for s in segments):
                return ""
        if not text or echoed_prompt(text):
            return ""
        return normalize_punct(text)


def main():
    # Windows 下 stdout 默认是 GBK：中文识别结果会直接抛 UnicodeEncodeError
    # （协议流是 UTF-8 JSON，宿主也按 UTF-8 读）。必须显式改过来。
    sys.stdout.reconfigure(encoding="utf-8")
    model_name = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_MODEL
    try:
        engine = Engine(model_name)
    except Exception as exc:  # noqa: BLE001
        emit({"error": f"加载模型 {model_name} 失败：{exc}"})
        return 1

    emit(
        {
            "ready": True,
            "model": model_name,
            "engine": engine.kind,
            "device": engine.device,
        }
    )
    for line in sys.stdin:
        path = line.strip()
        if not path:
            continue
        if path == "__quit__":
            break
        try:
            peak, rms = audio_level(path)
            text = engine.transcribe(path)
            emit({"text": text, "quiet": (not text) and rms < 60 and peak < 500})
        except Exception as exc:  # noqa: BLE001
            emit({"error": f"转写失败：{exc}"})
    return 0


if __name__ == "__main__":
    sys.exit(main())
