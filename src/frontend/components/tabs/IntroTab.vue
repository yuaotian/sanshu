<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { NGrid, NGridItem } from 'naive-ui'
import FeatureCard, { type Feature } from '../common/FeatureCard.vue'
import ProjectInfoCard from '../common/ProjectInfoCard.vue'

const loading = ref(true)
const features = ref<Feature[]>([])

onMounted(async () => {
  // 模拟加载延迟，展示骨架屏体验
  await new Promise(resolve => setTimeout(resolve, 600))
  
  features.value = [
    {
      icon: 'i-carbon-chat text-xl text-blue-600 dark:text-blue-400',
      title: 'Zhi 智能交互',
      titleClass: 'text-blue-600 dark:text-blue-400',
      subtitle: '审时度势，智在必行',
      iconWrapperClass: 'bg-blue-50 dark:bg-blue-900/30',
      features: [
        '智能代码审查交互网关',
        '多模态输入（文本/图片/选项）',
        'KISS / YAGNI / SOLID 质量把控',
        '实时任务状态可视化',
        '防止 AI 幻觉与自作主张',
      ],
    },
    {
      icon: 'i-carbon-data-base text-xl text-purple-600 dark:text-purple-400',
      title: 'Ji 记忆管理',
      titleClass: 'text-purple-600 dark:text-purple-400',
      subtitle: '博闻强记，温故知新',
      iconWrapperClass: 'bg-purple-50 dark:bg-purple-900/30',
      features: [
        '项目级全局记忆存储',
        '对话上下文自动回忆',
        '智能规则去重与分类',
        '规则/偏好/模式/上下文管理',
        '长期开发习惯自动对齐',
      ],
    },
    {
      icon: 'i-carbon-search text-xl text-emerald-600 dark:text-emerald-400',
      title: 'Sou 语义搜索',
      titleClass: 'text-emerald-600 dark:text-emerald-400',
      subtitle: '搜神索隐，洞若观火',
      iconWrapperClass: 'bg-emerald-50 dark:bg-emerald-900/30',
      features: [
        '基于 acemcp 的语义索引',
        '实时文件变更监听',
        '智能等待（防止读取旧代码）',
        '自然语言代码定位',
        '多语言/多文件类型支持',
      ],
    },
    {
      icon: 'i-carbon-document text-xl text-orange-600 dark:text-orange-400',
      title: 'Context7 文档',
      titleClass: 'text-orange-600 dark:text-orange-400',
      subtitle: '博采众长，与时俱进',
      iconWrapperClass: 'bg-orange-50 dark:bg-orange-900/30',
      features: [
        '主流框架最新官方文档查询',
        '支持 Next.js / React / Vue 等',
        '智能降级与模糊搜索',
        'API 密钥配置支持',
        '避免知识库过时问题',
      ],
    },
    {
      icon: 'i-carbon-paint-brush text-xl text-pink-600 dark:text-pink-400',
      title: 'UI/UX Pro Max',
      titleClass: 'text-pink-600 dark:text-pink-400',
      subtitle: '万技归一，随用随载',
      iconWrapperClass: 'bg-pink-50 dark:bg-pink-900/30',
      features: [
        '智能设计系统生成',
        '技术栈推荐与分析',
        '样式代码语义搜索',
        '持久化生成结果配置',
        '自定义素材域名限制',
      ],
    },
    {
      icon: 'i-carbon-image text-xl text-indigo-600 dark:text-indigo-400',
      title: 'Tu 图标工坊',
      titleClass: 'text-indigo-600 dark:text-indigo-400',
      subtitle: '以图会意，取用自如',
      iconWrapperClass: 'bg-indigo-50 dark:bg-indigo-900/30',
      features: [
        'Iconfont 图标聚合搜索',
        'SVG / PNG 批量下载',
        '实时预览与样式调整',
        '自定义保存路径管理',
        '本地图标缓存机制',
      ],
    },
    {
      icon: 'i-carbon-magic-wand text-xl text-cyan-600 dark:text-cyan-400',
      title: 'Enhance 增强',
      titleClass: 'text-cyan-600 dark:text-cyan-400',
      subtitle: '化繁为简，精准表达',
      iconWrapperClass: 'bg-cyan-50 dark:bg-cyan-900/30',
      features: [
        '提示词智能优化与重写',
        '利用代码上下文增强语义',
        '历史对话意图整合',
        '结构化专业提示词生成',
        '模糊指令清晰化处理',
      ],
    },
    {
      icon: 'i-carbon-settings text-xl text-gray-600 dark:text-gray-400',
      title: '个性化设置',
      titleClass: 'text-gray-600 dark:text-gray-400',
      subtitle: '掌控全局，随心所欲',
      iconWrapperClass: 'bg-gray-50 dark:bg-gray-800/50',
      features: [
        '深色/浅色主题与字体定制',
        '窗口置顶/尺寸/固定模式',
        'Telegram 远程通知推送',
        '自定义音效与快捷键',
        '智能代理策略管理',
      ],
    },
  ]
  loading.value = false
})
</script>

<template>
  <div class="tab-content pb-6">
    <!-- 项目信息卡片 -->
    <div class="mb-5">
      <ProjectInfoCard />
    </div>

    <!-- 功能卡片区域 -->
    <n-grid
      :cols="4"
      :x-gap="16"
      :y-gap="16"
      item-responsive
      responsive="screen"
    >
      <!-- 骨架屏占位 (加载时显示) -->
      <template v-if="loading">
        <n-grid-item v-for="n in 8" :key="n" span="4 s:2 m:2 l:1">
          <FeatureCard :loading="true" />
        </n-grid-item>
      </template>

      <!-- 真实数据 (加载后显示) -->
      <template v-else>
        <n-grid-item
          v-for="(feature, index) in features"
          :key="index"
          span="4 s:2 m:2 l:1"
        >
          <FeatureCard :feature="feature" />
        </n-grid-item>
      </template>
    </n-grid>
  </div>
</template>
