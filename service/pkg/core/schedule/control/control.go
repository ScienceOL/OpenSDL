package control

import (
	// 外部依赖
	"context"
	"errors"
	"fmt"
	"sync"
	"time"

	haxmap "github.com/alphadose/haxmap"
	gin "github.com/gin-gonic/gin"
	websocket "github.com/gorilla/websocket"
	melody "github.com/olahol/melody"
	ants "github.com/panjf2000/ants/v2"
	r "github.com/redis/go-redis/v9"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	constant "github.com/scienceol/opensdl/service/pkg/common/constant"
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	notify "github.com/scienceol/opensdl/service/pkg/core/notify"
	events "github.com/scienceol/opensdl/service/pkg/core/notify/events"
	schedule "github.com/scienceol/opensdl/service/pkg/core/schedule"
	lab "github.com/scienceol/opensdl/service/pkg/core/schedule/lab"
	edge "github.com/scienceol/opensdl/service/pkg/core/schedule/lab/edge"
	auth "github.com/scienceol/opensdl/service/pkg/middleware/auth"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	redis "github.com/scienceol/opensdl/service/pkg/middleware/redis"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	eStore "github.com/scienceol/opensdl/service/pkg/repo/environment"
	mStore "github.com/scienceol/opensdl/service/pkg/repo/material"
	s "github.com/scienceol/opensdl/service/pkg/repo/sandbox"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

var (
	ctl  *control
	once sync.Once
)

const (
	registryPeriod = 1 * time.Second
	poolSize       = 200
)

type control struct {
	wsClient      *melody.Melody               // websocket 连接控制
	scheduleName  string                       // 调度器名
	labMap        *haxmap.Map[int64, lab.Edge] // lab 信息
	rClient       *r.Client                    // redis client
	pools         *ants.Pool                   // 任务池
	boardEvent    notify.MsgCenter             // 广播系统
	sandbox       repo.Sandbox                 // 脚本运行沙箱
	labStore      repo.LaboratoryRepo          // 实验室存储
	materialStore repo.MaterialRepo            // 物料调度
}

func NewControl(ctx context.Context) schedule.Control {
	once.Do(func() {
		wsClient := melody.New()
		wsClient.Config.MaxMessageSize = constant.MaxMessageSize
		wsClient.Config.PingPeriod = 10 * time.Second
		scheduleName := fmt.Sprintf("lab-schedule-name-%s", uuid.NewV4().String())
		logger.Infof(ctx, "====================schedule name: %s ======================", scheduleName)

		ctl = &control{
			wsClient:      wsClient,
			scheduleName:  scheduleName,
			rClient:       redis.GetClient(),
			labMap:        haxmap.New[int64, lab.Edge](),
			labStore:      eStore.New(),
			materialStore: mStore.NewMaterialImpl(),
			boardEvent:    events.NewEvents(),
			sandbox:       s.NewSandbox(),
		}
		ctl.pools, _ = ants.NewPool(poolSize)
		ctl.initWebSocket(ctx)
	})

	return ctl
}

// edge 连接 websocket，第一时间接收到连接消息
func (i *control) Connect(ctx context.Context) {
	// edge 侧用户 websocket 连接
	ginCtx := ctx.(*gin.Context)
	labUser := auth.GetCurrentUser(ctx)

	edgeSession := ginCtx.GetHeader("EdgeSession")

	setSuccess, err := i.rClient.SetNX(ctx,
		utils.LabHeartName(labUser.LabUUID),
		edgeSession,
		100*utils.LabHeartTime-time.Second).Result()
	if err != nil {
		logger.Errorf(ctx, "schedule control set lab heart fail uuid: %s, err: %+v", labUser.LabUUID, err)
		common.ReplyErr(ginCtx, code.ParamErr.WithMsgf("set lab heart err: %+v", err))
		return
	}

	if !setSuccess {
		logger.Warnf(ctx, "schedule control lab already connect uuid: %s", labUser.LabUUID)
		common.ReplyErr(ginCtx, code.ParamErr.WithMsg("lab already exist"))
		return
	}

	defer func() {
		if _, err := i.rClient.Del(context.Background(),
			utils.LabHeartName(labUser.LabUUID)).Result(); err != nil {
			logger.Errorf(ctx, "schedule control lab already connet uuid: %s", labUser.LabUUID)
		}
	}()

	if err := i.wsClient.HandleRequestWithKeys(ginCtx.Writer, ginCtx.Request, map[string]any{
		"ctx":          ctx,
		"lab_id":       labUser.LabID,
		"lab_uuid":     labUser.LabUUID,
		"lab_user_id":  labUser.ID,
		"edge_session": edgeSession,
	}); err != nil {
		logger.Errorf(ctx, "schedule control HandleRequestWithKeys fail err: %+v", err)
	}
}

// init websocket
func (i *control) initWebSocket(ctx context.Context) {
	// 连接正式创建
	i.wsClient.HandleConnect(func(s *melody.Session) {
		labID := s.MustGet("lab_id").(int64)
		labUUID := s.MustGet("lab_uuid").(uuid.UUID)
		sessionCtx := s.MustGet("ctx").(*gin.Context)
		labUserID := s.MustGet("lab_user_id").(string)
		edgeSession := s.MustGet("edge_session").(string)
		labInfo := &lab.LabInfo{
			UUID:        labUUID,
			ID:          labID,
			LabUserID:   labUserID,
			Session:     s,
			EdgeSession: edgeSession,
		}

		edgeImpl, err := edge.NewLab(sessionCtx, labInfo)
		if err != nil {
			s.CloseWithMsg(fmt.Appendf(nil, "create lab instance fail err: %+v", err))
			return
		}

		if oldEdgeImpl, ok := i.labMap.Get(labID); ok {
			oldEdgeImpl.Close(sessionCtx)
		}

		i.labMap.Set(labID, edgeImpl)
	})

	// edge websocket 断开
	i.wsClient.HandleClose(func(s *melody.Session, _ int, _ string) error {
		// 关闭之后的回调
		labID := s.MustGet("lab_id").(int64)
		ctx := s.MustGet("ctx").(*gin.Context)
		if edgeImpl, ok := i.labMap.GetAndDel(labID); ok && edgeImpl != nil {
			edgeImpl.Close(ctx)
		}

		return nil
	})

	// edge 资源回收
	i.wsClient.HandleDisconnect(func(s *melody.Session) {
		labID := s.MustGet("lab_id").(int64)
		ctx := s.MustGet("ctx").(*gin.Context)
		if edgeImpl, ok := i.labMap.GetAndDel(labID); ok && edgeImpl != nil {
			edgeImpl.Close(ctx)
		}
	})

	i.wsClient.HandleError(func(s *melody.Session, err error) {
		// 读或写或写 buf 满了出错
		if errors.Is(err, melody.ErrMessageBufferFull) {
			return
		}
		if closeErr, ok := err.(*websocket.CloseError); ok {
			if closeErr.Code == websocket.CloseGoingAway {
				return
			}
		}

		if ctx, ok := s.Get("ctx"); ok {
			logger.Infof(ctx.(context.Context), "schedule control initWebSocket websocket find HandleError keys: %+v, err: %+v", s.Keys, err)
		}
	})

	i.wsClient.HandleMessage(func(s *melody.Session, b []byte) {
		labID := s.MustGet("lab_id").(int64)
		sessionCtx := s.MustGet("ctx").(*gin.Context)
		edgeImpl, ok := i.labMap.Get(labID)
		if !ok {
			logger.Errorf(sessionCtx, "can not get lab impl lab id: %d", labID)
			return
		}

		edgeImpl.OnEdgeMessge(sessionCtx, s, b)
	})

	count := 0
	i.wsClient.HandlePong(func(s *melody.Session) {
		count++
		if count%500 == 0 {
			labID := s.MustGet("lab_id").(int64)
			sessionCtx := s.MustGet("ctx").(*gin.Context)
			edgeImpl, ok := i.labMap.Get(labID)
			if !ok {
				logger.Errorf(sessionCtx, "can not get lab impl lab id: %d", labID)
				return
			}
			edgeImpl.OnPongMessage(sessionCtx)
		}
	})
}

// 关闭清理资源
func (i *control) Close(ctx context.Context) {
	if i.wsClient != nil {
		if err := i.wsClient.CloseWithMsg([]byte("reboot")); err != nil {
			logger.Errorf(ctx, "Close fail CloseWithMsg err: %+v", err)
		}
	}

	i.labMap.ForEach(func(i int64, e lab.Edge) bool {
		e.Close(ctx)
		return true
	})

	if i.pools != nil {
		i.pools.Release()
	}
}
