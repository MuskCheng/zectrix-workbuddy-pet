use crate::model::{Status, Task};

/// An actual transition, never a heartbeat or a replayed historical record.
pub fn transition(before: Option<Status>, after: Status) -> bool {
    before != Some(after) && matches!(after, Status::Waiting | Status::Done)
}

/// 是否该为这个事件响铃。`allow_chime=false` 用于首次扫描：那次读到的多半是
/// 历史状态的回放，逐个响会把旧事件当新通知。首扫之后由调用方对「当前正停在
/// 等你处理」补一次提醒——设备超时重连后等待页会无声出现，那才是真漏报。
pub fn should_chime(allow_chime: bool, before: Option<Status>, after: Status) -> bool {
    allow_chime && transition(before, after)
}

/// 「已提醒过」的指纹，用来保证同一次等待/完成只响一次——包括跨进程重启、
/// 设备超时重连之后，这样「补提醒」不会在每次重连时重复吵人。
/// 等待态用「任务 + 上屏文案」：文案就是那次确认卡或提问的摘要，换一个提示
/// 自然换一个指纹；重复上报的同一条等待（进度、活动文字抖动）不会被当成新的。
pub fn notice_sig(task: &Task) -> Option<String> {
    match task.status() {
        Status::Waiting => Some(format!("{}|w|{}", task.key(), task.event.detail)),
        // 完成态用记录序号（文件绝对偏移，跨重启稳定）：新完成才算新提醒。
        Status::Done => Some(format!("{}|d|{}", task.key(), task.event.seq)),
        _ => None,
    }
}
pub fn still_relevant(task: &Task, state: Status) -> bool {
    !task.stale && task.status() == state
}
/// 设备固件对单个播放流的接收队列只有 6 帧（360 ms）；实测超过 5 帧即返回
/// 「处理失败」（RESULT 结束原因 7），与音量、发送节奏无关。因此单条提示音
/// 必须压在 300 ms 以内 —— 用「短促双音」而不是长旋律来表达状态。
pub const MAX_TONE_MS: usize = 300;

/// Locally synthesized 16 kHz mono PCM16. Short envelopes prevent clicks.
pub fn pcm(state: Status) -> Vec<u8> {
    let notes: &[(f32, usize)] = match state {
        // 两声短「嘀」：需要你回来处理
        Status::Waiting => &[(880.0, 80), (0.0, 40), (880.0, 80)],
        // 上行三连音：任务完成
        Status::Done => &[(659.25, 60), (783.99, 60), (1046.5, 90)],
        // 下行两音：出错或中途中止
        _ => &[(440.0, 90), (349.23, 90)],
    };
    let mut bytes = Vec::new();
    for &(frequency, milliseconds) in notes {
        let count = milliseconds * 16;
        for i in 0..count {
            let attack = (i as f32 / 160.0).min(1.0);
            let release = ((count - i) as f32 / 480.0).min(1.0);
            let phase = std::f32::consts::TAU * frequency * i as f32 / 16000.0;
            let sample =
                ((phase.sin() + 0.15 * (2.0 * phase).sin()) * attack * release * 14000.0) as i16;
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
    }
    bytes
}
pub fn wav(state: Status) -> Vec<u8> {
    let pcm = pcm(state);
    let length = pcm.len() as u32;
    let mut bytes = Vec::with_capacity(pcm.len() + 44);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(length + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&16000u32.to_le_bytes());
    bytes.extend_from_slice(&32000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&pcm);
    bytes
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chimes_only_on_transition_and_are_short_distinct_pcm() {
        assert!(transition(Some(Status::Working), Status::Waiting));
        assert!(transition(Some(Status::Working), Status::Done));
        assert!(!transition(Some(Status::Waiting), Status::Waiting));
        assert!(!transition(Some(Status::Done), Status::Done));
        assert!(!transition(Some(Status::Waiting), Status::Working));
        // 首扫一律静默（历史回放），之后的真实变化才响。
        assert!(!should_chime(false, Some(Status::Working), Status::Waiting));
        assert!(!should_chime(false, None, Status::Done));
        assert!(should_chime(true, Some(Status::Working), Status::Waiting));
        assert!(should_chime(true, Some(Status::Working), Status::Done));
        assert!(!should_chime(true, Some(Status::Waiting), Status::Waiting));
        assert_ne!(pcm(Status::Waiting), pcm(Status::Done));
        for status in [Status::Waiting, Status::Done, Status::Error] {
            let bytes = pcm(status);
            // 设备单流上限：300 ms = 4800 采样 = 9600 字节；短于 60 ms 会被当成异常帧
            assert!(
                bytes.len() <= MAX_TONE_MS * 32,
                "{status:?} 提示音超过设备单流上限：{} 字节",
                bytes.len()
            );
            assert!(bytes.len() >= 1920, "{status:?} 提示音过短");
            assert_eq!(bytes.len() % 2, 0);
            let encoded = wav(status);
            assert_eq!(&encoded[..4], b"RIFF");
            assert_eq!(&encoded[8..16], b"WAVEfmt ");
            assert_eq!(&encoded[24..28], &16000u32.to_le_bytes());
            assert_eq!(&encoded[44..], &bytes);
        }
    }

    fn task(id: &str, status: Status, detail: &str, seq: u64) -> Task {
        Task {
            event: crate::model::Event {
                task_id: id.into(),
                thread_id: None,
                seq,
                status,
                title: "整理今天的待办".into(),
                detail: detail.into(),
                progress: None,
                progress_source: crate::model::ProgressSource::Agent,
                progress_steps: None,
                agent: "WorkBuddy".into(),
            },
            source: "workbuddy".into(),
            updated_at: 1_700_000_000,
            stale: false,
        }
    }

    #[test]
    fn notice_signature_is_stable_for_one_prompt_and_changes_with_the_next() {
        // 同一条等待被重复上报（进度、活动文字抖动）时指纹不变：不该反复响。
        let waiting = task(
            "t1",
            Status::Waiting,
            "有写入操作等你确认 · 请回电脑点允许",
            5,
        );
        let again = task(
            "t1",
            Status::Waiting,
            "有写入操作等你确认 · 请回电脑点允许",
            9,
        );
        assert_eq!(notice_sig(&waiting), notice_sig(&again));
        // 换成另一张确认卡/另一个提问 → 必须重新提醒。
        assert_ne!(
            notice_sig(&waiting),
            notice_sig(&task("t1", Status::Waiting, "要删除 3 个文件，允许吗？", 6))
        );
        // 换一个任务 → 必须重新提醒。
        assert_ne!(
            notice_sig(&waiting),
            notice_sig(&task(
                "t2",
                Status::Waiting,
                "有写入操作等你确认 · 请回电脑点允许",
                5
            ))
        );
        // 完成态按事件序号：同一事件跨重启不重复响，新完成会响。
        assert_eq!(
            notice_sig(&task("t1", Status::Done, "", 7)),
            notice_sig(&task("t1", Status::Done, "", 7))
        );
        assert_ne!(
            notice_sig(&task("t1", Status::Done, "", 7)),
            notice_sig(&task("t1", Status::Done, "", 8))
        );
        // 工作中的状态不是提醒点。
        assert_eq!(
            notice_sig(&task("t1", Status::Working, "在跑测试", 7)),
            None
        );
    }
}
