package reagent

import (
	// 外部依赖
	"context"
	"strings"
	"time"

	gorm "gorm.io/gorm"
	
	// 内部引用
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	model "github.com/scienceol/opensdl/service/pkg/model"
)

type reagentImpl struct {
	repo.IDOrUUIDTranslate
}

func NewReagentRepo() repo.ReagentRepo {
	return &reagentImpl{IDOrUUIDTranslate: repo.NewBaseDB()}
}

// activeScope 返回一个 GORM Scope，用于过滤掉已软删除的记录
func activeScope(db *gorm.DB) *gorm.DB {
	return db.Where("is_deleted = ?", 0)
}

func (r *reagentImpl) UpdateReagentByUUID(ctx context.Context, id uuid.UUID, data map[string]interface{}) error {
	// 确保更新时不会操作已删除的记录
	db := r.DBWithContext(ctx).Model(&model.Reagent{}).Scopes(activeScope).Where("uuid = ?", id)
	if err := db.Updates(data).Error; err != nil {
		logger.Errorf(ctx, "UpdateReagentByUUID failed: %v", err)
		return code.UpdateDataErr.WithErr(err)
	}
	return nil
}

func (r *reagentImpl) ConsumeStock(ctx context.Context, id uuid.UUID, consumption float64) error {
	db := r.DBWithContext(ctx)
	res := db.Model(&model.Reagent{}).Scopes(activeScope).
		Where("uuid = ?", id).
		UpdateColumn("stock_in_quantity", gorm.Expr("stock_in_quantity - ?", consumption))
	if res.Error != nil {
		logger.Errorf(ctx, "ConsumeStock err: %v", res.Error)
		return code.ReagentInsufficientErr.WithErr(res.Error)
	}
	if res.RowsAffected == 0 {
		return code.RecordNotFound
	}
	return nil
}

func (r *reagentImpl) ListReagents(ctx context.Context, q repo.ReagentQuery) ([]*model.Reagent, int64, error) {
	db := r.DBWithContext(ctx).Model(&model.Reagent{}).Scopes(activeScope)

	// Filter by lab_uuid if provided
	db = db.Where("lab_id = ?", q.LabID)

	orConds := make([]string, 0, 3)
	args := make([]interface{}, 0, 3)

	if q.NameLike != nil && *q.NameLike != "" {
		orConds = append(orConds, "name ILIKE ?")
		args = append(args, "%"+*q.NameLike+"%")
	}
	if q.CAS != nil && *q.CAS != "" {
		orConds = append(orConds, "cas = ?")
		args = append(args, *q.CAS)
	}
	if q.Supplier != nil && *q.Supplier != "" {
		orConds = append(orConds, "supplier ILIKE ?")
		args = append(args, "%"+*q.Supplier+"%")
	}

	if len(orConds) > 0 {
		db = db.Where("("+strings.Join(orConds, " OR ")+")", args...)
	}

	if q.BeforeExp != nil && *q.BeforeExp {
		db = db.Where("expiry_date IS NULL OR expiry_date >= ?", time.Now().Truncate(24*time.Hour))
	}
	// 库存状态过滤
	if q.Stock != nil {
		switch *q.Stock {
		case repo.StockSufficient:
			db = db.Where("stock_in_quantity <> 0")
		default:
			db = db.Where("stock_in_quantity = 0")
		}
	}
	// 有效性过滤
	if q.Valid != nil {
		now := time.Now().Truncate(24 * time.Hour)
		switch *q.Valid {
		case repo.ValidValid:
			db = db.Where("expiry_date IS NULL OR expiry_date >= ?", now)
		case repo.ValidExpired:
			db = db.Where("expiry_date IS NOT NULL AND expiry_date < ?", now)
		}
	}

	var total int64
	if err := db.Count(&total).Error; err != nil {
		return nil, 0, code.QueryRecordErr.WithErr(err)
	}

	order := q.OrderBy

	if q.Limit == 0 {
		q.Limit = 20
	}

	list := make([]*model.Reagent, 0, q.Limit)
	if err := db.Order(order).Offset(q.Offset).Limit(q.Limit).Find(&list).Error; err != nil {
		return nil, 0, code.QueryRecordErr.WithErr(err)
	}
	return list, total, nil
}
