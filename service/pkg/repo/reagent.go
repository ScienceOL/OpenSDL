package repo

import (
	// 外部依赖
	"context"

	// 内部引用
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	model "github.com/scienceol/opensdl/service/pkg/model"
)

// 试剂库存状态（整型枚举）
type StockStatus int8

const (
	StockSufficient   StockStatus = 1 // 充足（> 0）
	StockInsufficient StockStatus = 2 // 不足（= 0）
)

// 试剂有效性状态（整型枚举）
type ValidStatus int8

const (
	ValidValid   ValidStatus = 1 // 有效（expiry_date 为空或 >= 今天）
	ValidExpired ValidStatus = 2 // 失效（expiry_date < 今天）
)

// SortOrder 定义排序方向
type SortOrder string

const (
	SortOrderAsc  SortOrder = "asc"  // 升序
	SortOrderDesc SortOrder = "desc" // 降序
)

// ReagentQuery 过滤条件
type ReagentQuery struct {
	LabID     int64
	NameLike  *string
	CAS       *string
	Supplier  *string
	BeforeExp *bool        // 兼容旧字段：仅查询未过期（expiry_date >= today）
	Stock     *StockStatus // 充足/不足
	Valid     *ValidStatus // 有效/失效
	OrderBy   string       // 默认 id desc
	Offset    int
	Limit     int
}

type ReagentRepo interface {
	IDOrUUIDTranslate

	// 根据 UUID 更新，data 只包含需要更新的字段
	UpdateReagentByUUID(ctx context.Context, uuid uuid.UUID, data map[string]interface{}) error
	// 库存扣减：stock_in_quantity = stock_in_quantity - consumption
	ConsumeStock(ctx context.Context, uuid uuid.UUID, consumption float64) error
	// 列表
	ListReagents(ctx context.Context, q ReagentQuery) ([]*model.Reagent, int64, error)
}
