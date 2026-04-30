pub(crate) const EMOJI: &[(&str, &str)] = &[
    // Faces & emotions
    ("smile", "😊"),
    ("smiley", "😃"),
    ("grin", "😁"),
    ("joy", "😂"),
    ("rofl", "🤣"),
    ("sweat_smile", "😅"),
    ("laughing", "😆"),
    ("wink", "😉"),
    ("heart_eyes", "😍"),
    ("kissing_heart", "😘"),
    ("kissing", "😗"),
    ("yum", "😋"),
    ("stuck_out_tongue", "😛"),
    ("sunglasses", "😎"),
    ("nerd", "🤓"),
    ("monocle", "🧐"),
    ("thinking", "🤔"),
    ("zipper_mouth", "🤐"),
    ("raised_eyebrow", "🤨"),
    ("neutral_face", "😐"),
    ("expressionless", "😑"),
    ("smirk", "😏"),
    ("unamused", "😒"),
    ("roll_eyes", "🙄"),
    ("grimacing", "😬"),
    ("lying_face", "🤥"),
    ("relieved", "😌"),
    ("pensive", "😔"),
    ("sleepy", "😪"),
    ("sleeping", "😴"),
    ("mask", "😷"),
    ("nauseated", "🤢"),
    ("sneezing", "🤧"),
    ("hot_face", "🥵"),
    ("cold_face", "🥶"),
    ("woozy", "🥴"),
    ("dizzy_face", "😵"),
    ("exploding_head", "🤯"),
    ("cowboy", "🤠"),
    ("partying", "🥳"),
    ("money_mouth", "🤑"),
    ("hugs", "🤗"),
    ("shush", "🤫"),
    ("salute", "🫡"),
    ("hand_over_mouth", "🤭"),
    ("confused", "😕"),
    ("worried", "😟"),
    ("slightly_frowning", "🙁"),
    ("frowning", "😦"),
    ("anguished", "😧"),
    ("open_mouth", "😮"),
    ("astonished", "😲"),
    ("flushed", "😳"),
    ("pleading", "🥺"),
    ("cry", "😢"),
    ("sob", "😭"),
    ("scream", "😱"),
    ("disappointed", "😞"),
    ("weary", "😩"),
    ("tired_face", "😫"),
    ("yawning", "🥱"),
    ("triumph", "😤"),
    ("rage", "😡"),
    ("angry", "😠"),
    ("skull", "💀"),
    ("ghost", "👻"),
    ("alien", "👽"),
    ("robot", "🤖"),
    ("poop", "💩"),
    ("clown", "🤡"),
    // Gestures & hands
    ("wave", "👋"),
    ("raised_hand", "✋"),
    ("ok_hand", "👌"),
    ("thumbsup", "👍"),
    ("+1", "👍"),
    ("thumbsdown", "👎"),
    ("-1", "👎"),
    ("clap", "👏"),
    ("pray", "🙏"),
    ("handshake", "🤝"),
    ("point_right", "👉"),
    ("point_left", "👈"),
    ("point_down", "👇"),
    ("point_up", "☝️"),
    ("raised_hands", "🙌"),
    ("muscle", "💪"),
    ("v", "✌️"),
    ("crossed_fingers", "🤞"),
    ("vulcan_salute", "🖖"),
    ("metal", "🤘"),
    ("call_me", "🤙"),
    ("writing_hand", "✍️"),
    ("fist", "✊"),
    ("punch", "👊"),
    ("pinched_fingers", "🤌"),
    ("middle_finger", "🖕"),
    // Hearts & symbols
    ("heart", "❤️"),
    ("orange_heart", "🧡"),
    ("yellow_heart", "💛"),
    ("green_heart", "💚"),
    ("blue_heart", "💙"),
    ("purple_heart", "💜"),
    ("black_heart", "🖤"),
    ("broken_heart", "💔"),
    ("sparkling_heart", "💖"),
    ("two_hearts", "💕"),
    ("star", "⭐"),
    ("star2", "🌟"),
    ("sparkles", "✨"),
    ("boom", "💥"),
    ("fire", "🔥"),
    ("100", "💯"),
    ("check", "✅"),
    ("x", "❌"),
    ("warning", "⚠️"),
    ("question", "❓"),
    ("exclamation", "❗"),
    ("zzz", "💤"),
    ("speech_balloon", "💬"),
    ("thought_balloon", "💭"),
    ("eyes", "👀"),
    // Celebration & misc
    ("tada", "🎉"),
    ("confetti", "🎊"),
    ("trophy", "🏆"),
    ("medal", "🥇"),
    ("rocket", "🚀"),
    ("computer", "💻"),
    ("phone", "📱"),
    ("lock", "🔒"),
    ("key", "🔑"),
    ("bulb", "💡"),
    ("hammer", "🔨"),
    ("wrench", "🔧"),
    ("bug", "🐛"),
    ("pizza", "🍕"),
    ("beer", "🍺"),
    ("coffee", "☕"),
    ("wine", "🍷"),
    ("cake", "🎂"),
    ("dog", "🐶"),
    ("cat", "🐱"),
    ("penguin", "🐧"),
    ("snake", "🐍"),
    ("crab", "🦀"),
    ("unicorn", "🦄"),
    ("dragon", "🐉"),
    ("rainbow", "🌈"),
    ("sun", "☀️"),
    ("moon", "🌙"),
    ("earth", "🌍"),
    ("snowflake", "❄️"),
];

pub(crate) enum TextSpan {
    Text(String),
    Emoji(String),
}

pub(crate) fn is_emoji_char(c: char) -> bool {
    let u = c as u32;
    matches!(u,
        0x00A9 | 0x00AE | 0x203C | 0x2049 | 0x2122 | 0x2139 |
        0x2194..=0x2199 | 0x21A9..=0x21AA |
        0x231A..=0x231B | 0x2328 | 0x23CF | 0x23E9..=0x23F3 | 0x23F8..=0x23FA |
        0x24C2 | 0x25AA..=0x25AB | 0x25B6 | 0x25C0 | 0x25FB..=0x25FE |
        0x2600..=0x2604 | 0x260E | 0x2611 | 0x2614..=0x2615 | 0x2618 | 0x261D |
        0x2620 | 0x2622..=0x2623 | 0x2626 | 0x262A | 0x262E..=0x262F |
        0x2638..=0x263A | 0x2640 | 0x2642 | 0x2648..=0x2653 |
        0x265F..=0x2660 | 0x2663 | 0x2665..=0x2666 | 0x2668 |
        0x267B | 0x267E..=0x267F | 0x2692..=0x2697 | 0x2699 |
        0x269B..=0x269C | 0x26A0..=0x26A1 | 0x26A7 | 0x26AA..=0x26AB |
        0x26B0..=0x26B1 | 0x26BD..=0x26BE | 0x26C4..=0x26C5 |
        0x26CE..=0x26CF | 0x26D1 | 0x26D3..=0x26D4 | 0x26E9..=0x26EA |
        0x26F0..=0x26F5 | 0x26F7..=0x26FA | 0x26FD |
        0x2702 | 0x2705 | 0x2708..=0x270D | 0x270F | 0x2712 | 0x2714 |
        0x2716 | 0x271D | 0x2721 | 0x2728 | 0x2733..=0x2734 | 0x2744 |
        0x2747 | 0x274C | 0x274E | 0x2753..=0x2755 | 0x2757 |
        0x2763..=0x2764 | 0x2795..=0x2797 | 0x27A1 | 0x27B0 | 0x27BF |
        0x2934..=0x2935 | 0x2B05..=0x2B07 | 0x2B1B..=0x2B1C | 0x2B50 | 0x2B55 |
        0x3030 | 0x303D | 0x3297 | 0x3299 |
        0x1F000..=0x1FAFF
    )
}

/// Convert an emoji character cluster to its Twemoji CDN PNG URL.
pub(crate) fn emoji_to_twemoji_url(emoji: &str) -> String {
    let codepoints: Vec<String> = emoji.chars()
        .filter(|&c| c != '\u{FE0F}') // strip variation selector-16
        .map(|c| format!("{:x}", c as u32))
        .collect();
    format!(
        "https://cdnjs.cloudflare.com/ajax/libs/twemoji/14.0.2/72x72/{}.png",
        codepoints.join("-")
    )
}

/// Split a string into alternating plain-text and emoji cluster spans.
pub(crate) fn split_emoji(text: &str) -> Vec<TextSpan> {
    let mut spans: Vec<TextSpan> = Vec::new();
    let mut current_text = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if is_emoji_char(c) {
            if !current_text.is_empty() {
                spans.push(TextSpan::Text(std::mem::take(&mut current_text)));
            }
            let mut seq = String::from(c);
            i += 1;
            // Consume continuation chars: VS-16, ZWJ, skin-tone modifiers, tags, keycap
            while i < chars.len() {
                let u = chars[i] as u32;
                if matches!(u,
                    0xFE0F | 0xFE0E | 0x20E3 | 0x200D |
                    0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F
                ) {
                    seq.push(chars[i]);
                    i += 1;
                    // After ZWJ, consume the joined emoji too
                    if u == 0x200D && i < chars.len() && is_emoji_char(chars[i]) {
                        seq.push(chars[i]);
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            spans.push(TextSpan::Emoji(seq));
        } else {
            current_text.push(c);
            i += 1;
        }
    }
    if !current_text.is_empty() {
        spans.push(TextSpan::Text(current_text));
    }
    spans
}

pub(crate) fn find_matches(prefix: &str) -> Vec<String> {
    let lower = prefix.to_lowercase();
    EMOJI.iter()
        .filter(|(name, _)| name.starts_with(lower.as_str()))
        .map(|(_, ch)| ch.to_string())
        .collect()
}
