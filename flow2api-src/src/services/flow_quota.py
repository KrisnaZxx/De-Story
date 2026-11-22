"""Only explicit account quota rejections permit switching generation accounts."""


def is_quota_error(error):
    text = str(error).lower()
    return any(marker in text for marker in (
        "insufficient_credit", "insufficient credit", "not enough credit",
        "credit_balance_too_low", "credits_exhausted", "credit_limit_exceeded",
        "public_error_insufficient", "public_error_quota_exceeded",
        "public_error_not_enough_credits", "public_error_model_quota_exceeded",
        "public_error_account_quota_exceeded", "public_error_daily_quota_exceeded",
        "daily limit exceeded", "daily quota exceeded", "quota_exhausted",
        "余额不足", "积分不足", "配额耗尽",
    ))
