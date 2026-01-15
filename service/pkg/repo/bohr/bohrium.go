package bohr

import (
	// 外部依赖
	"context"
	"errors"
	"net/http"
	"strconv"

	resty "github.com/go-resty/resty/v2"
	gorm "gorm.io/gorm"

	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	model "github.com/scienceol/opensdl/service/pkg/model"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

type AccessKeyUserInfo struct {
	UserID int64 `json:"userId"`
	OrgID  int64 `json:"orgId"`
}

type BohrUserInfo struct {
	ID              int64  `json:"id"`
	Status          int    `json:"status"`
	Email           string `json:"email"`
	Name            string `json:"name"`
	NickName        string `json:"nickname"`
	NickNameEn      string `json:"nicknameEn"`
	Phone           string `json:"phone"`
	Kind            int    `json:"kind"`
	PhoneVerify     int    `json:"phoneVerify"`
	Oversea         int    `json:"oversea"`
	AreaCode        int    `json:"areaCode"`
	UserNo          string `json:"userNo"`
	ActivityId      int    `json:"activityId"`
	UtmCampaign     string `json:"utmCampaign"`
	CourseApply     bool   `json:"courseApply"`
	MemberRole      int    `json:"memberRole"`
	LoginSource     string `json:"loginSource"`
	UtmSource       string `json:"utmSource"`
	RegisterChannel string `json:"registerChannel"`
	ActivityUserId  int64  `json:"activityUserId"`
	IsBindWechat    bool   `json:"isBindWechat"`
	IsSetPwd        bool   `json:"isSetPwd"`
	WeChatNickname  string `json:"weChatNickname"`
	Avatar          string `json:"avatar"`
}

type BohrUserProfileInfo struct {
	Name     string `json:"name"`
	ID       int64  `json:"id"`
	Avatar   string `json:"profilePhoto"`
	Nickname string `json:"nickname"`
	Phone    string `json:"phone"`
	Status   int    `json:"status"`
	UserNo   string `json:"userNo"`
	Email    string `json:"email"`
}

type BohrImpl struct {
	account  *resty.Client
	bohrCore *resty.Client
	repo.IDOrUUIDTranslate
}

func New() repo.Account {
	conf := config.Global().RPC.Bohr
	accountConf := config.Global().RPC.Account
	return &BohrImpl{
		account: resty.New().
			EnableTrace().
			SetBaseURL(accountConf.Addr),
		IDOrUUIDTranslate: repo.NewBaseDB(),
		bohrCore: resty.New().
			EnableTrace().
			SetBaseURL(conf.Addr),
	}
}

func (b *BohrImpl) CreateLabUser(ctx context.Context, user *model.LabInfo) error {
	panic("not impl")
}

func (b *BohrImpl) GetLabUserInfo(ctx context.Context, req *model.LabAkSk) (*model.UserData, error) {
	// 实验室用户就是创建该实验室的人
	labData := &model.Laboratory{}
	if err := b.DBWithContext(ctx).
		Where("access_key = ? and access_secret = ?",
			req.AccessKey, req.AccessSecret).
		Select("id", "user_id", "uuid").
		Take(labData).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, code.RecordNotFound
		}

		logger.Errorf(ctx, "GetLabUserInfo fail err: %+v", err)
		return nil, err
	}

	return &model.UserData{
		ID:      labData.UserID,
		LabID:   labData.ID,
		LabUUID: labData.UUID,
	}, nil
}

func (b *BohrImpl) BatchGetUserInfo(ctx context.Context, userIDs []string) ([]*model.UserData, error) {
	paramUserIDs := utils.FilterSlice(userIDs, func(u string) (uint64, bool) {
		id, err := strconv.ParseUint(u, 10, 64)
		if err != nil {
			logger.Errorf(ctx, "BatchGetUserInfo can not translate user string to uint64 user: %s", u)
			return 0, false
		}
		return id, true
	})

	resData := &common.RespT[[]*BohrUserInfo]{}
	resp, err := b.account.R().
		SetContext(ctx).
		SetBody(map[string]any{
			"ids": paramUserIDs,
		}).
		SetResult(resData).Post("/account_api/users/list")
	if err != nil {
		logger.Errorf(ctx, "BatchGetUserInfo err: %+v user ids : %+v", err, userIDs)
		return nil, code.CasDoorQueryLabUserErr.WithMsg(err.Error())
	}

	if resp.StatusCode() != http.StatusOK {
		logger.Errorf(ctx, "BatchGetUserInfo http code: %d", resp.StatusCode())
		return nil, code.CasDoorQueryLabUserErr
	}

	if resData.Code != code.Success {
		logger.Errorf(ctx, "BatchGetUserInfo resp code not zero err msg:%+v", *resData.Error)
		return nil, code.BohrBatchQueryErr
	}

	return utils.FilterSlice(resData.Data, func(item *BohrUserInfo) (*model.UserData, bool) {
		return &model.UserData{
			Owner:             "",
			Name:              item.Name,
			ID:                strconv.FormatInt(item.ID, 10),
			Avatar:            item.Avatar,
			Type:              "",
			DisplayName:       item.NickName,
			SignupApplication: "",
			// Phone:             item.Phone,
			Phone: func() string {
				if len(item.Phone) >= 11 {
					return item.Phone[:3] + "*****" + item.Phone[8:]
				}
				return item.Phone
			}(),
			Status: item.Status,
			UserNo: item.UserNo,
			Email:  item.Email,
		}, true
	}), nil
}

func (b *BohrImpl) GetUserInfo(ctx context.Context, userID string) (*model.UserData, error) {
	respData := &common.RespT[*BohrUserProfileInfo]{}
	resp, err := b.account.R().
		SetContext(ctx).
		SetPathParam("id", userID).
		SetResult(respData).Get("/account_api/users/{id}")
	if err != nil {
		logger.Errorf(ctx, "GetUserInfo err: %+v user id : %+v", err, userID)
		return nil, code.RPCHttpErr.WithMsg(err.Error())
	}

	if resp.StatusCode() != http.StatusOK {
		logger.Errorf(ctx, "GetUserInfo http code: %d", resp.StatusCode())
		return nil, code.RPCHttpCodeErr.WithMsgf("GetUserInfo code: %+d", resp.StatusCode())
	}

	if respData.Code != code.Success {
		logger.Errorf(ctx, "GetUserInfo resp code not zero err msg:%+v", *respData.Error)
		return nil, code.RPCHttpCodeErr.WithMsgf("GetUserInfo resp code: +%d", respData.Code)
	}

	return &model.UserData{
		Owner:             "",
		Name:              respData.Data.Name,
		ID:                strconv.FormatInt(respData.Data.ID, 10),
		Avatar:            respData.Data.Avatar,
		Type:              "",
		DisplayName:       respData.Data.Nickname,
		SignupApplication: "",
		Phone:             respData.Data.Phone,
		Status:            respData.Data.Status,
		UserNo:            respData.Data.UserNo,
		Email:             respData.Data.Email,
	}, nil
}

func (b *BohrImpl) GetLabUserByAccessKey(ctx context.Context, accessKey string) (*model.UserData, error) {
	respData := &common.RespT[*AccessKeyUserInfo]{}
	resp, err := b.bohrCore.R().
		SetContext(ctx).
		SetQueryParam("accessKey", accessKey).
		SetResult(respData).
		Get("/api/v1/ak/get_user")
	if err != nil {
		logger.Errorf(ctx, "GetLabUserByAccessKey err: %+v", err)
		return nil, code.RPCHttpErr.WithMsg(err.Error())
	}

	if resp.StatusCode() != http.StatusOK {
		logger.Errorf(ctx, "GetLabUserByAccessKey http code: %d", resp.StatusCode())
		return nil, code.RPCHttpCodeErr.WithMsgf("GetLabUserByAccessKey code: %+d", resp.StatusCode())
	}

	if respData.Code != code.Success {
		logger.Errorf(ctx, "GetLabUserByAccessKey resp code not zero err msg:%+v", *respData.Error)
		return nil, code.RPCHttpCodeErr.WithMsgf("GetLabUserByAccessKey resp code: +%d", respData.Code)
	}

	return &model.UserData{
		Owner: strconv.FormatInt(respData.Data.OrgID, 10),
		ID:    strconv.FormatInt(respData.Data.UserID, 10),
	}, nil
}
