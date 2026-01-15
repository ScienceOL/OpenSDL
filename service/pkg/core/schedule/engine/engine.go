package engine

import (
	// 外部依赖
	"context"
	"time"

	// 内部引用
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
)

/*
	调度引擎模块，抽象调度接口
*/

type Task interface {
	Run() error                                           // 运行
	Stop() error                                          // 停止
	GetStatus(ctx context.Context) error                  //  获取状态
	OnJobUpdate(ctx context.Context, data *JobData) error // edge 侧状态更新
	Type(ctx context.Context) JobType                     // 获取任务类型
	ID(ctx context.Context) uuid.UUID                     // 获取当前任务 id

	// 状态控制
	GetDeviceActionStatus(ctx context.Context, key ActionKey) (ActionValue, bool)                // 获取设备状态更新
	SetDeviceActionStatus(ctx context.Context, key ActionKey, free bool, needMore time.Duration) // 设置设备状态
	InitDeviceActionStatus(ctx context.Context, key ActionKey, start time.Time, free bool)       // 初始化设备状态
	DelStatus(ctx context.Context, key ActionKey)                                                // 删除设备状态
}
