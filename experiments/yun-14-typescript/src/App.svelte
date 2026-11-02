<script lang="ts">
  let { n }: { n: number } = $props();
  let s: string = $state("a");
  const bad: number = s; // わざと入れた型エラー
</script>
<p>{n}{bad}</p>
