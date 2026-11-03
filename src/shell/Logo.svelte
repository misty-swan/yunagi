<script lang="ts">
  // ロゴ：アイコン（24px）＋小文字の「yunagi」（デザインガイド 8章）。
  // アイコンはアプリアイコン（夕凪版）を小さくした絵なので、画面の色のトークンではなく絵の色で描く
  // （テーマで変えない）。
  // 絵の中の ID は、ロゴを2つ同時に出しても重ならないよう、部品ごとに変える。
  const uid = $props.id();
  const sky = `${uid}-sky`;
  const sea = `${uid}-sea`;
  const round = `${uid}-round`;
  const above = `${uid}-above`;
</script>

<span class="logo">
  <svg class="mark" viewBox="0 0 24 24" aria-hidden="true">
    <defs>
      <linearGradient id={sky} x1="0" y1="0" x2="0" y2="1">
        <stop offset="0" stop-color="#2B2542" />
        <stop offset="0.34" stop-color="#4A3550" />
        <stop offset="0.54" stop-color="#D88A5E" />
        <stop offset="0.58" stop-color="#EFA567" />
      </linearGradient>
      <linearGradient id={sea} x1="0" y1="0" x2="0" y2="1">
        <stop offset="0" stop-color="#1A2236" />
        <stop offset="1" stop-color="#0F1526" />
      </linearGradient>
      <clipPath id={round}><rect width="24" height="24" rx="6" /></clipPath>
      <clipPath id={above}><rect width="24" height="13.92" /></clipPath>
    </defs>
    <g clip-path="url(#{round})">
      <rect width="24" height="13.92" fill="url(#{sky})" />
      <rect y="13.92" width="24" height="10.08" fill="url(#{sea})" />
      <circle cx="12" cy="15.4" r="4.32" fill="#F6B57A" clip-path="url(#{above})" />
      <rect x="3.6" y="13.92" width="16.8" height="1" fill="rgba(255,236,220,0.85)" />
    </g>
  </svg>
  <span class="word">yunagi</span>
</span>

<style>
  .logo {
    display: inline-flex;
    align-items: center;
    gap: calc(10 * var(--u));
  }

  .mark {
    flex: none;
    width: calc(24 * var(--u));
    height: calc(24 * var(--u));
  }

  .word {
    font-size: calc(17 * var(--u));
    font-weight: 500;
    letter-spacing: 0.05em;
  }
</style>
