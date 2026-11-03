//! 項目の並び順のキー（item の `sort_key`。分数インデックス。データモデル設計書 3章 item）。
//!
//! キーは文字列の大小で並ぶ。2つのキーの間に、ほかの項目のキーを変えずに新しいキーを作れる
//! （並べ替え・間への追加で、動かす項目の行だけを書けばよい）。
//!
//! # キーの形
//!
//! - 文字は `0`〜`9`・`A`〜`Z`・`a`〜`z` の 62 種（ASCII の順に並ぶ）。キーを 0 以上 1 未満の
//!   62 進の小数の、小数点より後ろの桁とみなす（`V` は 31/62 ≒ 0.5）。
//! - 空でなく、最後の桁が `0` でない。最後が `0` のキーは、1つ短いキーと同じ値になって、
//!   その間にキーを作れないため。
//!
//! この形に当てはまらないキー（[`is_valid`] が `false`）や、同じキーの隣どうしの間には作れない。
//! そのときは、呼び出し側がその日のキーを [`spread`] で付け直す。

/// キーに使う文字（ASCII の順）。
const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

fn digit(c: u8) -> Option<usize> {
    DIGITS.iter().position(|&d| d == c)
}

/// キーの形（上記）に当てはまるか。
pub fn is_valid(key: &str) -> bool {
    !key.is_empty() && !key.ends_with('0') && key.bytes().all(|c| digit(c).is_some())
}

/// `before` と `after` の間に入るキー。`before` が `None` なら先頭（`after` より前）、`after` が
/// `None` なら末尾（`before` より後）、両方 `None` なら最初のキー。
///
/// どちらかが形に当てはまらないか、`before` が `after` 以上なら `None`（作れない）。
pub fn between(before: Option<&str>, after: Option<&str>) -> Option<String> {
    if before.is_some_and(|k| !is_valid(k)) || after.is_some_and(|k| !is_valid(k)) {
        return None;
    }
    if let (Some(b), Some(a)) = (before, after)
        && b >= a
    {
        return None;
    }
    Some(midpoint(
        before.unwrap_or("").as_bytes(),
        after.map(str::as_bytes),
    ))
}

/// `n` 個のキーを、先頭から順に並ぶように作る（その日のキーの付け直しに使う）。
/// 同じ桁数の値を 0 と 1 の間に等しい間隔で置くので、どのキーの間にも後から入れられる。
pub fn spread(n: usize) -> Vec<String> {
    let base = DIGITS.len() as u128;
    // n 個を、隣と 1 以上離して置ける桁数
    let mut len = 1;
    let mut span = base;
    while span <= n as u128 + 1 {
        len += 1;
        span *= base;
    }
    let step = span / (n as u128 + 1);
    (1..=n as u128)
        .map(|i| {
            let mut v = i * step;
            let mut digits = vec![b'0'; len];
            for d in digits.iter_mut().rev() {
                *d = DIGITS[(v % base) as usize];
                v /= base;
            }
            // 最後の 0 を除いても値は同じ（キーの形に合わせる）
            while digits.last() == Some(&b'0') {
                digits.pop();
            }
            String::from_utf8(digits).expect("keys are ASCII")
        })
        .collect()
}

/// `a`（空なら 0）と `b`（`None` なら 1）の間の値の桁。`a < b` で、どちらも最後の桁が `0` でない。
fn midpoint(a: &[u8], b: Option<&[u8]>) -> String {
    if let Some(b) = b {
        // 共通の頭の桁はそのまま使う（`a` の足りない桁は 0 とみなす）
        let n = b
            .iter()
            .enumerate()
            .take_while(|&(i, &c)| a.get(i).copied().unwrap_or(b'0') == c)
            .count();
        if n > 0 {
            let head = std::str::from_utf8(&b[..n]).expect("keys are ASCII");
            let a_rest = a.get(n..).unwrap_or(&[]);
            return format!("{head}{}", midpoint(a_rest, Some(&b[n..])));
        }
    }
    let da = a.first().map_or(0, |&c| digit(c).expect("valid key"));
    let db = b.map_or(DIGITS.len(), |b| digit(b[0]).expect("valid key"));
    if db - da > 1 {
        // 1桁で間に入る
        let mid = (da + db).div_ceil(2);
        return char::from(DIGITS[mid]).to_string();
    }
    // 1桁の差しか無い：b の頭の1桁が b より短くて a より大きければそれを使い、
    // そうでなければ a の頭の桁に続けて、a の残りと 1 の間を作る
    if let Some(b) = b
        && b.len() > 1
    {
        return char::from(b[0]).to_string();
    }
    let a_rest = a.get(1..).unwrap_or(&[]);
    format!("{}{}", char::from(DIGITS[da]), midpoint(a_rest, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(before: Option<&str>, after: Option<&str>) -> String {
        let k = between(before, after).unwrap();
        assert!(is_valid(&k), "{k}");
        if let Some(b) = before {
            assert!(b < k.as_str(), "{b} < {k}");
        }
        if let Some(a) = after {
            assert!(k.as_str() < a, "{k} < {a}");
        }
        k
    }

    #[test]
    fn makes_keys_between_neighbours() {
        assert_eq!(check(None, None), "V");
        assert_eq!(check(Some("V"), None), "l");
        assert_eq!(check(None, Some("V")), "G");
        assert_eq!(check(Some("a"), Some("b")), "aV");
        assert_eq!(check(Some("a"), Some("a1")), "a0V");
        assert_eq!(check(Some("az"), Some("b")), "azV");
        assert_eq!(check(Some("a"), Some("aV")), "aG");
        assert_eq!(check(Some("z"), None), "zV");
        assert_eq!(check(None, Some("1")), "0V");
        assert_eq!(check(Some("A1"), Some("B")), "AW");
        assert_eq!(check(Some("A"), Some("C")), "B");
    }

    #[test]
    fn rejects_invalid_or_unordered_neighbours() {
        for (b, a) in [
            (Some("a0"), None),
            (None, Some("")),
            (Some("a-"), None),
            (Some("b"), Some("a")),
            (Some("a"), Some("a")),
            (Some("あ"), None),
        ] {
            assert_eq!(between(b, a), None, "{b:?} {a:?}");
        }
    }

    #[test]
    fn repeated_insertions_stay_ordered() {
        // 先頭・末尾・同じ所への挿入を繰り返しても、いつも間に作れる
        let mut keys = vec![check(None, None)];
        for i in 0..300 {
            let k = match i % 3 {
                0 => check(None, Some(&keys[0])),
                1 => check(Some(keys.last().unwrap()), None),
                _ => {
                    let mid = keys.len() / 2;
                    check(Some(&keys[mid - 1]), Some(&keys[mid]))
                }
            };
            keys.push(k);
            keys.sort();
        }
        let mut sorted = keys.clone();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len());
        // 同じ所へ 100 回入れても、キーは長くなりすぎない
        assert!(keys.iter().all(|k| k.len() < 120), "{keys:?}");
    }

    #[test]
    fn spread_makes_increasing_short_keys() {
        assert!(spread(0).is_empty());
        assert_eq!(spread(1), ["V"]);
        for n in [2, 61, 62, 200, 5000] {
            let keys = spread(n);
            assert_eq!(keys.len(), n);
            assert!(keys.windows(2).all(|w| w[0] < w[1]), "{n}");
            assert!(keys.iter().all(|k| is_valid(k) && k.len() <= 3), "{n}");
            // 先頭の前・末尾の後・隣どうしの間に作れる
            check(None, Some(&keys[0]));
            check(Some(&keys[n - 1]), None);
            check(Some(&keys[0]), Some(&keys[1]));
        }
    }
}
