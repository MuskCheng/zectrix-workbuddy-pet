"""常驻转写进程：加载一次模型，之后按行收 WAV 路径、回一行 JSON。

为什么不直接用 `whisper file.wav`：CLI 每次都要重新 import torch、加载模型
（实测冷启动 6～8 秒），而语音输入是「松手就要看见字」的场景，等不起。
常驻进程只在第一次启动时付这笔钱，之后每段音频只剩推理时间。

协议（只用 stdout，一行一个 JSON）：
    输入：WAV 的绝对路径；`__quit__` 表示退出。
    输出：{"ready": true, "model": "..."} / {"text": "..."} / {"error": "..."}

模型的进度条与警告都走 stderr，不会污染协议流。
"""
import json
import sys

# 引导它吐标点的提示词。副作用是「没有语音内容时它会把这句原样吐回来」，
# 所以下面要按它做一次回声过滤——否则误触录音会变成往对话框里塞这一行。
GUIDE = "以下是普通话的句子，请使用简体中文并加标点。"
# 顺带塞几个开发场景的高频词：small 模型对「纪要/记要」这类同音词常写错，
# 给了词表就能拉回来（实测「会议纪要」从「计要/记要」被纠正）。
HOTWORDS = "常用词：会议纪要、待办、代码、编译、分支、提交、接口、部署、需求、文档。"
PROMPT = GUIDE + HOTWORDS

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


def clean_text(result):
    """把「其实什么都没说」的几种情况收敛成空串。

    whisper 对纯噪声/静音会做两件事之一：输出空串，或者回显 initial_prompt。
    后者如果直接当识别结果，就会在安静环境里往对话里粘一句莫名其妙的话。
    """
    text = (result.get("text") or "").strip()
    if not text:
        return ""
    segments = result.get("segments") or []
    # 所有分段都被判为「不是语音」：这是环境噪声，不是人话。
    if segments and all(s.get("no_speech_prob", 0.0) > 0.5 for s in segments):
        return ""

    def norm(s):
        return "".join(ch for ch in s if ch.isalnum())

    if norm(text) and norm(text) in norm(PROMPT):
        return ""
    return normalize_punct(text)


def main():
    # Windows 下 stdout 默认是 GBK：中文识别结果会直接抛 UnicodeEncodeError
    # （协议流是 UTF-8 JSON，宿主也按 UTF-8 读）。必须显式改过来。
    sys.stdout.reconfigure(encoding="utf-8")
    model_name = sys.argv[1] if len(sys.argv) > 1 else "small"
    try:
        import whisper
    except Exception as exc:  # 缺包时给出结构化错误，宿主好原样报给用户
        emit({"error": f"无法导入 whisper：{exc}"})
        return 1

    try:
        model = whisper.load_model(model_name)
    except Exception as exc:
        emit({"error": f"加载模型 {model_name} 失败：{exc}"})
        return 1

    emit({"ready": True, "model": model_name})
    for line in sys.stdin:
        path = line.strip()
        if not path:
            continue
        if path == "__quit__":
            break
        try:
            # initial_prompt 只为了让它吐标点：whisper 的中文默认输出常常
            # 是一长串没有断句的字，粘进对话里还得自己加。
            result = model.transcribe(
                path,
                language="zh",
                initial_prompt=PROMPT,
                condition_on_previous_text=False,
                verbose=None,
            )
            emit({"text": clean_text(result)})
        except Exception as exc:
            emit({"error": f"转写失败：{exc}"})
    return 0


if __name__ == "__main__":
    sys.exit(main())
