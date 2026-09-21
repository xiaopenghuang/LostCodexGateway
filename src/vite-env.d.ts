/// <reference types="vite/client" />

/** 本项目自定义的环境变量 */
interface ImportMetaEnv {
  /**
   * 开发期「有数据状态」视觉检查的场景名。
   * 取值：verified | suspect | connecting | error
   * 仅在 dev 下生效，生产构建会因 `import.meta.env.DEV` 为 false 而被摇掉。
   */
  readonly VITE_LCFG_FIXTURE?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
