"""已确认业务规则；与技能版本一起保存，进行中的任务使用原文件。"""
from decimal import Decimal

RESULT_SCHEMA = 1
CALCULATION_VERSION = '1.0.0'
CLASSIFICATION_VERSION = 'v2'
BUSINESS_TIMEZONE = 'Asia/Shanghai'
SEGMENT_DAYS = 31
BASELINE_VALID_DAYS = 14
LOOKBACK_LIMIT_DAYS = 366
MAIN_QUERY_LIMIT = 1000
PROCESS_TIMEOUT_SECONDS = 120
JOB_WAIT_SECONDS = 30
MAX_SOURCE_BYTES = 16 * 1024 * 1024
ACCURACY_THRESHOLDS = tuple(map(Decimal, ('0.8', '0.7', '0.6')))
REVENUE_THRESHOLDS = tuple(map(Decimal, ('0.95', '0.9', '0.85', '0.8', '0.7', '0.6')))
ORDER_PASS_RATE = Decimal('0.8')
REVENUE_PASS_RATE = Decimal('0.85')
COMPARABLE_PERCENTAGE_POINTS = Decimal('5')
ANOMALY_RATIO = Decimal('0.15')
TEN_THOUSAND = Decimal('10000')
WORST_SHOP_COUNT = 3
TOP_ITEM_COUNT = 5
FRESH_CLASSES = ('鲜猪', '鲜鸡', '鲜虾', '鲜鱼', '鲜牛')
NON_FRESH = '非鲜品'
UNKNOWN = '未知'
CONFLICT = '冲突'
METRICS = {
    'predictOrderAccuracy': ('预估订货量准确率', 'suggestedPredictOrderNum', 'multipliedActualOrderNum'),
    'manualOrderAccuracy': ('人工订货量准确率', 'orderNum', 'multipliedActualOrderNum'),
    'predictConsumeAccuracy': ('预估耗用量准确率', 'predictConsumeNum', 'actualConsumeNum'),
    'inventoryAccuracy': ('库存准确率', 'theoryInventory', 'arrivalBeforeActualInventory'),
}
DATA_FILES = {
    'shops': 'shops.json', 'indicators': 'indicators.json', 'orders': 'orders.json',
    'revenue': 'revenue.json', 'dailyInventory': 'daily-inventory.json', 'items': 'items.json',
    'groups': 'groups.json', 'classifications': 'classifications.json', 'inventoryHistory': 'inventory-history.json',
    'freshConfig': 'fresh-config.json', 'dishConsume': 'dish-consume.json',
}
OP_PREFIX = 'supply-chain-server.predict.'
OPERATIONS = {
    'shops': 'shops.read', 'indicators': 'indicators.read', 'orders': 'orders.read',
    'revenue': 'revenue.read', 'dailyInventory': 'daily-inventory.read', 'items': 'items.read',
    'groups': 'item-groups.read', 'classifications': 'classifications.read',
    'inventoryHistory': 'inventory-history.read', 'freshConfig': 'fresh-config.read',
    'dishConsume': 'dish-consume.read', 'warehouse': 'warehouse.read',
    'shopTeams': 'shop-teams.read', 'shopTeamTree': 'shop-team-tree.read', 'partitionTree': 'partition-tree.read',
}

MAX_READ_ATTEMPTS = 2
RETRYABLE_READ_FAILURES = frozenset(('DEPENDENCY_UNAVAILABLE', 'RATE_LIMITED', 'RESOURCE_LIMIT', 'RESULT_EXPIRED'))
STATE_FILE = 'analysis-state.json'
BUNDLE_FILE = 'bundle.json'
REPORT_DIRECTORY = 'report'
ENTRY_FILE = 'SKILL.md'
RUN_RECORD = 'run.json'
RESOURCE_DIRECTORY = 'resources'
DATA_DIRECTORY = 'data'
SKILL_NAME = 'predict-accuracy-attribution'
CLIENT_DIRECTORIES = {'Codex': '.agents/skills', 'WorkBuddy': '.workbuddy/skills'}
SOURCE_LIVE = '真实受控接口读取'
SOURCE_LOCAL = '本地模拟接口，仅验证流程和计算'
