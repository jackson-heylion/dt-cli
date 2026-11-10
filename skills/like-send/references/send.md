# 点赞接口

系统 `hrmp`，按底座选定的环境读取[接口清单](../scripts/requirements.json)对应要求，以该环境当前真实resolve契约为准。使用本人原身份，不传供应链视角。由底座取得对应平台入口并登录；旧任务的PROFILE_REQUIRED与旧doctor的STG清单不作为新平台阻塞。

| 别名 | 原接口 | 输入／衔接 |
| --- | --- | --- |
| like-person-search | POST `/app/person/list` | search=原姓名，pageNo/pageSize；personId → beLikedPersonId |
| like-types | GET `/app/like/type/list` | 同一类型的 ID、名称、behaviorList → 发送字段 |
| like-sent-records | POST `/app/like/record/giftedByMe` | 只支持 pageNo/pageSize，查询本人历史 |
| like-send | POST `/app/like/record/add` | WRITE，完整点赞正文 |

业务成功 code=0；类型列表 /data，人员／历史列表 /data/list，总数 /data/total。外层 data.body 按底座解析。

## 人员、类型与草稿

姓名按原词搜索，用真实部门／职位和上下文消歧；不猜部门、拼音筛选或遍历全员。HRMP personId 不能替换为 IAM 员工 ID。

| 必填发送字段 | 来源 |
| --- | --- |
| beLikedPersonId | 已确认人员 personId |
| likeTypeId | 已选择类型 ID |
| likeTypeName | 同一类型名称 |
| likeBehaviorContent | 同一类型 behaviorList 中选定行为 |
| specificDeeds | 用户事实整理后的完整事迹 |

不附加操作者 ID。用户指定的类型保持原意；未指定且根据事迹只有一个合理匹配可选择并说明，否则读取真实类型和行为后集中补问。“其他”须真实存在于对应行为列表。personId和类型ID由Agent读取，记录名由底座管理，不要求用户提供这些技术标识。

人员与类型可独立读取；缺少事迹时仍可按原姓名读取少量人员并取得类型候选，完成后只补问事迹和确有歧义的业务选择。只起草时无需解析发送／历史接口。草稿用四项业务信息展示：接收人、类型、行为、完整内容，保留测试标记，注明环境及“已起草，尚未发送”。

## 本人历史

接口没有接收人或日期参数。需要筛选时，在本人历史和原预算内分页，按 recipientName／createTime 后筛；姓名不证明稳定人员身份，total 不是后筛总数。

原实现仅含本人非关联点赞，按 recordId 倒序，不证明全系统完整历史或 createTime 全局排序。不能因某页日期较早就提前宣称期间完整。

“最近”默认原序5条样本，展示真实 createTime，不附加30天窗口；全局时间排名须有完整覆盖。“本周／上周”按北京时间自然周；近N天含今天的N个自然日、上界为固定查询时刻，明确 N×24 小时则滚动计算。

未读完只交付已核对命中，不能断言期间全部或确无记录。历史表展示接收人、类型、行为、事迹及时间；行为为空写“未提供”，不从提交值补造或推断未保存。

## 发送与核对

按底座 `writes` 完成 prepare → inspect → confirm → dispatch；当前明确发送指令为授权依据。业务 code=0、contractMatched 和原写入成功终态均核对后，才报告“点赞已发送”及四项业务信息。

prepared／confirmed 不是派发成功。超时或结果未知只查原 recordId／intent，不能依据相似历史文本重发。事件行与最终信封用底座解析器处理；inspect 缺 stage 不代表失败。

只有用户要求发送后回读时才查本人历史，核对稳定业务编号、接收人、类型、内容和时间；同文或条数增加不能单独证明本轮新增。优先关联派发返回编号或已保存编号变化，证据不足说明无法确认具体新增记录，不能再发送。
