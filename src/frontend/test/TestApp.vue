<script setup lang="ts">
import { ref } from 'vue'
import ComponentsTest from './components/ComponentsTest.vue'
import MainLayoutTest from './components/MainLayoutTest.vue'
import McpPopupTest from './components/McpPopupTest.vue'
import PlanPanelTest from './components/PlanPanelTest.vue'
import ThemesTest from './components/ThemesTest.vue'

const activeTab = ref('main-ui')
const useSimulatedWindow = ref(true)

const tabs = [
  { key: 'main-ui', label: '主界面', component: MainLayoutTest },
  { key: 'mcp-popup', label: 'MCP 弹窗', component: McpPopupTest },
  { key: 'plan-panel', label: '执行计划', component: PlanPanelTest },
  { key: 'components', label: '组件库', component: ComponentsTest },
  { key: 'themes', label: '主题测试', component: ThemesTest },
]
</script>

<template>
  <n-message-provider>
    <div class="test-app">
      <div class="test-header">
        <h1>三术 - 组件样式测试环境</h1>
        <p class="test-description">
          独立的测试环境，用于开发和调试组件样式，直接引用真实组件
        </p>

        <!-- 模拟窗口开关 -->
        <div class="window-toggle">
          <n-switch v-model:value="useSimulatedWindow" size="small">
            <template #checked>
              模拟窗口 (600px)
            </template>
            <template #unchecked>
              全屏模式
            </template>
          </n-switch>
        </div>
      </div>

      <!-- 模拟窗口容器 -->
      <div v-if="useSimulatedWindow" class="simulated-window-container">
        <!-- 测试控制面板 -->
        <div class="test-controls">
          <n-card title="测试控制面板" size="small">
            <n-tabs v-model:value="activeTab" type="segment" size="small">
              <n-tab-pane
                v-for="tab in tabs"
                :key="tab.key"
                :name="tab.key"
                :tab="tab.label"
              />
            </n-tabs>
          </n-card>
        </div>

        <!-- 模拟窗口 -->
        <div class="simulated-window">
          <!-- 模拟窗口标题栏 -->
          <div class="window-titlebar">
            <div class="window-controls">
              <div class="window-control close" />
              <div class="window-control minimize" />
              <div class="window-control maximize" />
            </div>
            <div class="window-title">
              三术
            </div>
            <div class="window-spacer" />
          </div>

          <!-- 窗口内容 - 只显示真实的应用内容 -->
          <div class="window-content">
            <!-- 主界面内容 -->
            <div v-if="activeTab === 'main-ui'" class="app-content">
              <MainLayoutTest :show-controls="false" />
            </div>

            <!-- MCP弹窗内容 -->
            <div v-else-if="activeTab === 'mcp-popup'" class="app-content">
              <McpPopupTest :show-controls="false" />
            </div>

            <!-- 其他内容 -->
            <div v-else class="app-content">
              <component :is="tabs.find(t => t.key === activeTab)?.component" :show-controls="false" />
            </div>
          </div>
        </div>
      </div>

      <!-- 全屏模式 -->
      <div v-else class="fullscreen-mode">
        <!-- 标签页导航 -->
        <n-tabs v-model:value="activeTab" type="segment" size="small" justify-content="center">
          <n-tab-pane
            v-for="tab in tabs"
            :key="tab.key"
            :name="tab.key"
            :tab="tab.label"
          >
            <div class="tab-content">
              <component :is="tab.component" />
            </div>
          </n-tab-pane>
        </n-tabs>
      </div>
    </div>
  </n-message-provider>
</template>

<style scoped>
.test-app {
  min-height: 100vh;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  padding: 20px;
}

.test-header {
  text-align: center;
  margin-bottom: 30px;
}

.test-header h1 {
  color: white;
  margin-bottom: 10px;
  font-size: 2rem;
  text-shadow: 0 2px 4px rgba(0, 0, 0, 0.3);
}

.test-description {
  color: white;
  opacity: 0.9;
  font-size: 0.9rem;
  margin: 0 0 15px 0;
}

.window-toggle {
  display: flex;
  justify-content: center;
  margin-top: 15px;
}

/* 模拟窗口容器 */
.simulated-window-container {
  display: flex;
  gap: 20px;
  align-items: flex-start;
  min-height: calc(100vh - 200px);
  max-width: 1200px;
  margin: 0 auto;
}

/* 测试控制面板 */
.test-controls {
  flex: 0 0 300px;
  position: sticky;
  top: 20px;
}

.simulated-window {
  flex: 0 0 600px;
  background: var(--body-color, #ffffff);
  border-radius: 12px;
  box-shadow:
    0 20px 60px rgba(0, 0, 0, 0.3),
    0 8px 25px rgba(0, 0, 0, 0.2);
  overflow: hidden;
  transition: all 0.3s ease;
}

.simulated-window:hover {
  transform: translateY(-2px);
  box-shadow:
    0 25px 70px rgba(0, 0, 0, 0.35),
    0 10px 30px rgba(0, 0, 0, 0.25);
}

/* 窗口标题栏 */
.window-titlebar {
  height: 40px;
  background: linear-gradient(180deg, #f5f5f5 0%, #e8e8e8 100%);
  border-bottom: 1px solid #d0d0d0;
  display: flex;
  align-items: center;
  padding: 0 15px;
  user-select: none;
}

.window-controls {
  display: flex;
  gap: 8px;
}

.window-control {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  cursor: pointer;
  transition: all 0.2s ease;
}

.window-control.close {
  background: #ff5f57;
}

.window-control.minimize {
  background: #ffbd2e;
}

.window-control.maximize {
  background: #28ca42;
}

.window-control:hover {
  transform: scale(1.1);
  filter: brightness(1.1);
}

.window-title {
  flex: 1;
  text-align: center;
  font-size: 13px;
  font-weight: 500;
  color: #333;
}

.window-spacer {
  width: 68px; /* 平衡左侧控制按钮的宽度 */
}

/* 窗口内容 */
.window-content {
  background: var(--body-color, #ffffff);
  height: 900px;
  overflow-y: auto;
}

/* 应用内容 */
.app-content {
  width: 100%;
  height: 100%;
}

/* 全屏模式 */
.fullscreen-mode {
  background: var(--body-color, #ffffff);
  border-radius: 12px;
  padding: 20px;
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.1);
}

.tab-content {
  padding: 20px 0;
  animation: fadeIn 0.3s ease-in-out;
}

@keyframes fadeIn {
  from {
    opacity: 0;
    transform: translateY(10px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

/* 深色主题适配 */
.dark .window-titlebar {
  background: linear-gradient(180deg, #3a3a3a 0%, #2d2d2d 100%);
  border-bottom-color: #404040;
}

.dark .window-title {
  color: #e0e0e0;
}

/* 响应式设计 */
@media (max-width: 768px) {
  .simulated-window {
    width: 95%;
    margin: 0 10px;
  }

  .test-app {
    padding: 10px;
  }
}
</style>
