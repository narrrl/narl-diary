<script lang="ts">
  /*
   * The two lines above the list: which space is open, and where inside it the
   * list currently is. Both are clickable, because a tree is the one thing that
   * is genuinely faster to navigate with a pointer than with `:cd`.
   */
  import { hooks } from './commands'
  import { workspace } from './store.svelte'

  // The space is the first crumb, and it is already named by the bar above.
  const crumbs = $derived(workspace.path.slice(1))
</script>

<div class="spaces">
  {#each workspace.spaces as space (space.id)}
    <button
      class="space"
      class:on={space.id === workspace.spaceId}
      onclick={() => workspace.guard(() => workspace.enterSpace(space.id))}
    >
      {space.slug}
    </button>
  {/each}
  <button class="space add" title="new space (:space! <name>)" onclick={() => hooks.openCommandLine(':space! ')}>
    +
  </button>
</div>

{#if crumbs.length > 0}
  <div class="crumbs">
    <button onclick={() => workspace.spaceId !== null && workspace.guard(() => workspace.goTo(workspace.spaceId!))}>
      /
    </button>
    {#each crumbs as crumb, index (crumb.id)}
      <span class="faint">/</span>
      <button
        class:last={index === crumbs.length - 1}
        onclick={() => workspace.guard(() => workspace.goTo(crumb.id))}
      >
        {crumb.slug}
      </button>
    {/each}
  </div>
{/if}

<style>
  .spaces,
  .crumbs {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--line);
    flex: none;
    overflow-x: auto;
    white-space: nowrap;
  }

  .crumbs { font-size: 12px; }

  button {
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--fg-dim);
    padding: 0 6px;
    line-height: 18px;
  }

  button:hover { color: var(--fg); background: var(--bg-lift); }
  .space.on { color: var(--bg); background: var(--accent-dim); font-weight: 700; }
  .add { color: var(--fg-faint); }
  .crumbs .last { color: var(--accent); }

  @media (pointer: coarse) {
    button { padding: 4px 10px; }
  }
</style>
