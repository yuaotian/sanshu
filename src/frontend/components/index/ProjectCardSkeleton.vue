<script setup lang="ts">
// 项目卡片骨架屏组件
// 用于在加载项目列表时显示占位效果
</script>

<template>
  <div class="project-card-skeleton">
    <!-- 顶部装饰线占位 -->
    <div class="skeleton-top-border" />

    <div class="p-4 space-y-3">
      <!-- 头部：项目名称和状态 -->
      <div class="flex items-start justify-between gap-3">
        <div class="flex-1 min-w-0 space-y-2">
          <!-- 项目名称骨架 -->
          <div class="flex items-center gap-2">
            <div class="skeleton-box w-4 h-4 rounded" />
            <div class="skeleton-box w-32 h-4 rounded" />
          </div>
          <!-- 项目路径骨架 -->
          <div class="skeleton-box w-48 h-3 rounded" />
        </div>

        <!-- 状态徽章骨架 -->
        <div class="skeleton-box w-16 h-5 rounded-full" />
      </div>

      <!-- 文件统计骨架 -->
      <div class="flex items-center gap-3">
        <div class="skeleton-box w-12 h-4 rounded" />
        <div class="skeleton-box w-10 h-4 rounded" />
        <div class="skeleton-box w-10 h-4 rounded" />
      </div>

      <!-- 最后索引时间骨架 -->
      <div class="skeleton-box w-24 h-3 rounded" />

      <!-- 操作按钮骨架 -->
      <div class="flex items-center gap-2 pt-2 border-t border-gray-200/30 dark:border-gray-700/30">
        <div class="skeleton-box w-14 h-5 rounded" />
        <div class="flex-1" />
        <div class="skeleton-box w-12 h-6 rounded" />
        <div class="skeleton-box w-12 h-6 rounded" />
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 骨架屏卡片容器 */
.project-card-skeleton {
  position: relative;
  border-radius: 12px;
  overflow: hidden;
  border: 1px solid rgba(128, 128, 128, 0.15);
  background: var(--color-container, rgba(255, 255, 255, 0.5));
  backdrop-filter: blur(4px);
}

:root.dark .project-card-skeleton {
  background: rgba(24, 24, 28, 0.6);
}

/* 顶部装饰线骨架 */
.skeleton-top-border {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 2px;
  background: linear-gradient(
    90deg,
    transparent,
    rgba(128, 128, 128, 0.2),
    transparent
  );
}

/* 骨架块基础样式 */
.skeleton-box {
  background: linear-gradient(
    90deg,
    rgba(128, 128, 128, 0.1) 25%,
    rgba(128, 128, 128, 0.2) 50%,
    rgba(128, 128, 128, 0.1) 75%
  );
  background-size: 200% 100%;
  animation: skeleton-shimmer 1.5s ease-in-out infinite;
}

/* 深色模式骨架块 */
:root.dark .skeleton-box {
  background: linear-gradient(
    90deg,
    rgba(255, 255, 255, 0.05) 25%,
    rgba(255, 255, 255, 0.1) 50%,
    rgba(255, 255, 255, 0.05) 75%
  );
  background-size: 200% 100%;
}

/* 闪烁动画 */
@keyframes skeleton-shimmer {
  0% {
    background-position: 200% 0;
  }
  100% {
    background-position: -200% 0;
  }
}
</style>

