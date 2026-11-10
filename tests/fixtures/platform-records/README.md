# 平台签名记录夹具

沿用改造前 Job、Progress、Page 及 ExecutionResult 的字段序列，独立 HMAC-SHA256 生成。测试密钥为32个十进制42；不含真实凭证或业务数据。由公开 jobs status/results 验证，保护签名记录兼容及原始正文精度。
