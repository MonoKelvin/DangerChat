# 黄金测试集（本地）

**此目录用于存放黄金测试的真实微信截图，严禁提交到 Git。**

## 使用说明

1. 将真实微信截图（PNG 格式）放入此目录
2. 创建 `expected_regions.json` 标注文件，格式：
   ```json
   {
     "screenshot.png": {
       "size": [宽, 高],
       "regions": [
         {"tag": "input", "rect": [x, y, w, h]},
         {"tag": "send", "rect": [x, y, w, h]}
       ]
     }
   }
   ```
3. 运行 `pnpm test:golden` 执行黄金测试

## 测试逻辑

- 如果 `expected_regions.json` 不存在，测试自动跳过
- `.gitignore` 已配置忽略此目录下的所有 PNG 文件
- 黄金测试需要 `models/dc-layout-wechat` 模型权重
